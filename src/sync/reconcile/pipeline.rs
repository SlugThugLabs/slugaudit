//! The per-path reconciliation pipeline: for each dirty path, decide
//! delete / ignore / unchanged / re-index, then atomically commit the
//! resulting upserts and deletions under compare-and-swap. The barrier
//! loop that drives this pipeline lives in [`super::barrier`]; the SQL
//! it reads lives in [`super::queries`].

use super::queries::{query_current_parser_pack_version, query_existing_hashes};
use super::{ReconcileError, ReconcileOptions, ReconcileReport};
use crate::sync::discovery::{self, DiscoveredFile, sniff_kind};
use crate::sync::revision_skipped::{SKIP_REASON_OVERSIZE, SkippedRecord};
use crate::sync::{FileRecord, hash, manifest, revision, sample};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Builds the existence row for a dirty path over the import ceiling, and
/// records it only when the stored row does not already carry the same stat
/// identity — a watcher event on an unchanged oversize file must not churn a
/// revision. Compared against the hashes already read for the dirty set
/// rather than a fresh query.
fn push_skipped_record(
    records: &mut Vec<SkippedRecord>,
    existing_hashes: &HashMap<String, String>,
    path: &str,
    absolute_path: &Path,
) {
    let record = sample::skipped_record(path, absolute_path, SKIP_REASON_OVERSIZE);
    if existing_hashes.get(path) != Some(&record.identity.content_hash) {
        records.push(record);
    }
}

/// Reconciles dirty and deleted paths against the database.
///
/// For each dirty path, reads the file, computes its BLAKE3 hash, and
/// queries the database for the previously stored hash. If the hash
/// matches, the file is skipped (unchanged since last index). If the hash
/// differs or the file is new, it is sampled, parsed, analyzed, and added
/// to the upsert set. Deleted paths are added to the deletion set.
///
/// After processing all paths, the function calls `publish_revision` to
/// atomically commit the upserts and deletions under compare-and-swap.
///
/// # Errors
///
/// Returns an error if any file read, hash, sample, analysis, or database
/// operation fails.
#[cfg(test)]
pub(crate) fn reconcile_dirty_paths(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
) -> Result<ReconcileReport, ReconcileError> {
    let options = ReconcileOptions::for_sync(root, None);
    reconcile_dirty_paths_with_deadline(
        connection,
        root,
        dirty,
        deleted,
        expected_revision,
        &options,
        // No walk in hand: fixtures index every file they write, so the
        // indexed rows are the correct universe for these tests.
        &HashSet::new(),
    )
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
    existing_hashes: &HashMap<String, String>,
    skipped_records: &mut Vec<SkippedRecord>,
) -> Result<DirtyOutcome, ReconcileError> {
    let ReconcileOptions { limits, rules, .. } = options;
    let absolute_path = root.join(&path);

    let Some(metadata) = discovery::regular_file_metadata(&absolute_path)? else {
        // Gone, a directory (the watcher reports `mkdir` as a create), or a
        // symlink: nothing indexable lives here, so any row is stale.
        return Ok(if existing_hashes.contains_key(&path) {
            DirtyOutcome::Deleted(path)
        } else {
            DirtyOutcome::Ignored
        });
    };

    if let Some(rules) = rules
        && rules.should_ignore(&path)
    {
        return Ok(DirtyOutcome::Ignored);
    }

    let mtime = crate::util::mtime_unix_seconds(&metadata);
    // A size beyond `i64::MAX` cannot be stored in the SQLite integer
    // column, so it is skipped like an over-limit file (an existence row,
    // reported in `skipped`) instead of being saturated into a wrong size.
    let size = metadata.len();
    let byte_len = match i64::try_from(size) {
        Ok(len) if size <= limits.max_file_bytes => len,
        _ => {
            push_skipped_record(skipped_records, existing_hashes, &path, &absolute_path);
            return Ok(DirtyOutcome::Skipped);
        }
    };

    let identity = hash::hash_file(&path, &absolute_path)?;

    if let Some(existing_hash) = existing_hashes.get(&path)
        && existing_hash == &identity.content_hash
    {
        connection.execute(
            "UPDATE files SET modified_unix_seconds = ?1, byte_len = ?2 WHERE path = ?3",
            rusqlite::params![mtime, byte_len, path],
        )?;
        return Ok(DirtyOutcome::Unchanged);
    }

    let kind = sniff_kind(&absolute_path)?;
    let discovered = DiscoveredFile {
        relative_path: path.clone(),
        absolute_path: absolute_path.clone(),
        kind,
    };
    let sample = match sample::sample_file(&discovered, limits) {
        Ok(sample) => sample,
        Err(sample::SampleError::TooLarge { .. }) => {
            push_skipped_record(skipped_records, existing_hashes, &path, &absolute_path);
            return Ok(DirtyOutcome::Skipped);
        }
        Err(error) => return Err(error.into()),
    };
    let record = sample::to_file_record(sample, limits);
    Ok(DirtyOutcome::Upsert(Box::new(record)))
}

pub(crate) fn reconcile_dirty_paths_with_deadline(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
    options: &ReconcileOptions,
    discovered_paths: &HashSet<String>,
) -> Result<ReconcileReport, ReconcileError> {
    let mut upserts = Vec::new();
    let mut deletions = Vec::new();
    let mut skipped_records: Vec<SkippedRecord> = Vec::new();
    let mut reconciled = 0usize;
    let mut unchanged = 0usize;
    let mut ignored = 0usize;
    let mut skipped = 0usize;

    let existing_hashes = query_existing_hashes(connection, &dirty)?;

    for path in dirty {
        if let Some(elapsed) = options.deadline.exceeded() {
            return Err(ReconcileError::TimeBudgetExceeded {
                path: format!(" while processing {path}"),
                elapsed_ms: elapsed.as_millis(),
            });
        }
        match reconcile_single_dirty_path(
            connection,
            root,
            path,
            options,
            &existing_hashes,
            &mut skipped_records,
        )? {
            DirtyOutcome::Deleted(deleted_path) => deletions.push(deleted_path),
            DirtyOutcome::Ignored => ignored += 1,
            DirtyOutcome::Skipped => skipped += 1,
            DirtyOutcome::Unchanged => unchanged += 1,
            DirtyOutcome::Upsert(record) => {
                upserts.push(*record);
                reconciled += 1;
            }
        }
    }

    deletions.extend(deleted);

    if upserts.is_empty() && deletions.is_empty() && skipped_records.is_empty() {
        return Ok(ReconcileReport {
            reconciled,
            unchanged,
            deleted: 0,
            ignored,
            skipped,
        });
    }

    commit_reconcile_changes(
        connection,
        expected_revision,
        discovered_paths,
        &upserts,
        &deletions,
        &skipped_records,
    )?;

    Ok(ReconcileReport {
        reconciled,
        unchanged,
        deleted: deletions.len(),
        ignored,
        skipped,
    })
}

fn commit_reconcile_changes(
    connection: &mut Connection,
    expected_revision: Option<&str>,
    discovered_paths: &HashSet<String>,
    upserts: &[FileRecord],
    deletions: &[String],
    skipped_records: &[SkippedRecord],
) -> Result<(), ReconcileError> {
    let manifest_hash =
        manifest::compute_manifest_hash(connection, upserts, deletions, skipped_records)?;
    let parser_pack_version = query_current_parser_pack_version(connection)?;

    revision::publish_revision(
        connection,
        revision::RevisionInputs {
            expected_current: expected_revision,
            manifest_hash: &manifest_hash,
            parser_pack_version: &parser_pack_version,
            upserts,
            deletions,
            skipped: skipped_records,
            discovered_paths,
        },
    )?;
    Ok(())
}
