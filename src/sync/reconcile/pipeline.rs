//! The per-path reconciliation pipeline: for each dirty path, decide
//! delete / ignore / unchanged / re-index, then atomically commit the
//! resulting upserts and deletions under compare-and-swap. The barrier
//! loop that drives this pipeline lives in [`super::barrier`]; the SQL and
//! commit step it reads and writes through live in [`super::queries`].

use super::queries::{
    StoredState, commit_reconcile_changes, load_existing_hashes, touch_unchanged,
};
use super::{ReconcileError, ReconcileOptions, ReconcileReport};
use crate::model::{ResourceLimits, SourceIdentity};
use crate::sync::discovery::{self, DiscoveredFile, sniff_kind};
use crate::sync::revision_skipped::SkippedRecord;
use crate::sync::{FileRecord, hash, sample};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[cfg(test)]
pub(crate) use super::barrier::reconcile_dirty_paths;

enum DirtyOutcome {
    Deleted(String),
    Ignored,
    Skipped,
    Unchanged,
    /// Boxed: `FileRecord` carries the file's content and evidence, so an
    /// inline variant made every `DirtyOutcome` — including the four empty
    /// ones returned for a deleted, ignored, skipped or unchanged path —
    /// 256 bytes to move. Most dirty paths in a walk take one of those four,
    /// so the box pays for itself on the common case and costs one small
    /// allocation on the rare one.
    Upsert(Box<FileRecord>),
}

fn reconcile_single_dirty_path(
    connection: &mut Connection,
    root: &Path,
    path: String,
    options: &ReconcileOptions,
    stored: &StoredState,
    skipped_records: &mut Vec<SkippedRecord>,
) -> Result<DirtyOutcome, ReconcileError> {
    let existing_hashes = &stored.hashes;
    let absolute_path = root.join(&path);
    let Some(metadata) = discovery::regular_file_metadata(&absolute_path)? else {
        // Gone, a directory (the watcher reports `mkdir` as a create), or a
        // symlink: nothing indexable lives here, so any row is stale.
        return Ok(removed_or_ignored(&path, existing_hashes));
    };

    if ignored_by_rules(options, &path) {
        return Ok(DirtyOutcome::Ignored);
    }

    let mtime = crate::util::mtime_unix_seconds(&metadata);
    // A size beyond `i64::MAX` cannot be stored in the SQLite integer
    // column, so it is skipped like an over-limit file (an existence row,
    // reported in `skipped`) instead of being saturated into a wrong size.
    let Some(byte_len) = storable_byte_len(metadata.len(), &options.limits) else {
        sample::push_skipped_record(skipped_records, existing_hashes, &path, &absolute_path);
        return Ok(DirtyOutcome::Skipped);
    };

    let Some(identity) = hash_dirty_file(&path, &absolute_path, &options.limits)? else {
        // The file grew between the stat above and the bounded hash read.
        // Record it as an existence row exactly as the static oversize path
        // does, rather than indexing content the ceiling says not to read.
        sample::push_skipped_record(skipped_records, existing_hashes, &path, &absolute_path);
        return Ok(DirtyOutcome::Skipped);
    };

    // A `LoadFailed` row keeps its hash but is never "unchanged": its grammar
    // could not be fetched or loaded, so the parse is retried every sync.
    if !stored.retry.contains(&path)
        && existing_hash_matches(existing_hashes, &path, &identity.content_hash)
    {
        touch_unchanged(connection, &path, mtime, byte_len)?;
        return Ok(DirtyOutcome::Unchanged);
    }

    match indexable_record(&path, &absolute_path, options)? {
        Some(record) => Ok(DirtyOutcome::Upsert(Box::new(record))),
        None => {
            sample::push_skipped_record(skipped_records, existing_hashes, &path, &absolute_path);
            Ok(DirtyOutcome::Skipped)
        }
    }
}

fn removed_or_ignored(path: &str, existing_hashes: &HashMap<String, String>) -> DirtyOutcome {
    if existing_hashes.contains_key(path) {
        DirtyOutcome::Deleted(path.to_owned())
    } else {
        DirtyOutcome::Ignored
    }
}

fn ignored_by_rules(options: &ReconcileOptions, path: &str) -> bool {
    options
        .rules
        .as_ref()
        .is_some_and(|rules| rules.should_ignore(path))
}

fn storable_byte_len(size: u64, limits: &ResourceLimits) -> Option<i64> {
    let byte_len = i64::try_from(size).ok()?;
    (size <= limits.max_file_bytes).then_some(byte_len)
}

fn hash_dirty_file(
    path: &str,
    absolute_path: &Path,
    limits: &ResourceLimits,
) -> Result<Option<SourceIdentity>, ReconcileError> {
    match hash::hash_file(path, absolute_path, limits.max_file_bytes) {
        Ok(identity) => Ok(Some(identity)),
        Err(hash::HashError::TooLarge { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn existing_hash_matches(
    existing_hashes: &HashMap<String, String>,
    path: &str,
    content_hash: &str,
) -> bool {
    existing_hashes
        .get(path)
        .is_some_and(|existing| existing == content_hash)
}

/// Samples and analyzes one dirty file under the options' limits; its
/// parses run within what is left of the reconcile deadline.
fn indexable_record(
    path: &str,
    absolute_path: &Path,
    options: &ReconcileOptions,
) -> Result<Option<FileRecord>, ReconcileError> {
    let kind = sniff_kind(absolute_path)?;
    let discovered = DiscoveredFile {
        relative_path: path.to_owned(),
        absolute_path: absolute_path.to_path_buf(),
        kind,
    };
    let limits = &options.limits;
    match sample::sample_file(&discovered, limits) {
        Ok(sample) => Ok(Some(sample::to_file_record(
            sample,
            limits,
            &options.deadline,
        ))),
        Err(sample::SampleError::TooLarge { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// [`reconcile_dirty_paths`] under explicit [`ReconcileOptions`], checked
/// once per dirty path so a pathological set (a huge number of events, a
/// hung filesystem) fails closed with
/// [`ReconcileError::TimeBudgetExceeded`] instead of stalling the tool
/// call forever. The deadline is created by the caller so the whole
/// barrier-sync operation shares one budget across its iterations.
///
/// `discovered_paths` are the project-relative paths the caller's walk saw
/// on disk (the stat sweep's `on_disk`). Edge resolution treats them as
/// existing even when they were never indexed — an oversize-skipped file
/// that exists must not turn its importers into `Unresolved` dead-code
/// findings.
pub(crate) fn reconcile_dirty_paths_with_deadline(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
    options: &ReconcileOptions,
    discovered_paths: &HashSet<String>,
) -> Result<ReconcileReport, ReconcileError> {
    let stored = load_existing_hashes(connection, &dirty, options.deadline)?;
    let mut tally = ReconcileTally::default();
    let mut skipped_records: Vec<SkippedRecord> = Vec::new();

    for path in dirty {
        if let Some(elapsed) = options.deadline.exceeded() {
            return Err(ReconcileError::TimeBudgetExceeded {
                path: format!(" while processing {path}"),
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let outcome = reconcile_single_dirty_path(
            connection,
            root,
            path,
            options,
            &stored,
            &mut skipped_records,
        )?;
        tally.record(outcome);
    }

    tally.deletions.extend(deleted);

    if tally.upserts.is_empty() && tally.deletions.is_empty() && skipped_records.is_empty() {
        return Ok(tally.report());
    }

    commit_reconcile_changes(
        connection,
        expected_revision,
        discovered_paths,
        &tally.upserts,
        &tally.deletions,
        &skipped_records,
        options.deadline,
    )?;

    Ok(tally.report())
}

/// The per-path outcomes accumulated for one reconcile pass, and their
/// translation into the caller-visible [`ReconcileReport`].
#[derive(Default)]
struct ReconcileTally {
    upserts: Vec<FileRecord>,
    deletions: Vec<String>,
    reconciled: usize,
    unchanged: usize,
    ignored: usize,
    skipped: usize,
}

impl ReconcileTally {
    fn record(&mut self, outcome: DirtyOutcome) {
        match outcome {
            DirtyOutcome::Deleted(path) => self.deletions.push(path),
            DirtyOutcome::Ignored => self.ignored += 1,
            DirtyOutcome::Skipped => self.skipped += 1,
            DirtyOutcome::Unchanged => self.unchanged += 1,
            DirtyOutcome::Upsert(record) => {
                self.upserts.push(*record);
                self.reconciled += 1;
            }
        }
    }

    fn report(&self) -> ReconcileReport {
        ReconcileReport {
            reconciled: self.reconciled,
            unchanged: self.unchanged,
            deleted: self.deletions.len(),
            ignored: self.ignored,
            skipped: self.skipped,
        }
    }
}
