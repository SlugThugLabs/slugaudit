// slugaudit-line-exception: approved-by=slugthuglabs; reason=cohesive diff stage: stored-row comparison, classification-drift detection, and upsert/deletion delta construction
//! Diffs freshly sampled disk state against what a revision currently has
//! stored, and turns that diff into the upsert/deletion sets a publish
//! writes. Kept separate from `publish` itself so that module stays focused
//! on orchestrating the sample → diff → write sequence rather
//! than the mechanics of any one step.
use super::discovery;
use super::hash::aggregate_manifest_hash;
use super::manifest::{self, ChangeStatus, FileChange};
use super::publish::PublishError;
use super::revision::FileRecord;
use super::revision_skipped::{SKIP_REASON_STAT_FAILED, SkippedRecord};
use super::sample::{self, Sample, to_file_record};
use crate::model::{ResourceLimits, SourceIdentity};
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

mod load_failed;

pub(super) struct Diff {
    pub(super) changes: Vec<FileChange>,
    pub(super) manifest_hash: String,
    pub(super) added: usize,
    pub(super) modified: usize,
    pub(super) deleted: usize,
    pub(super) unchanged: usize,
}

/// Folds one source of skipped files (discovery skips, then sampling skips)
/// into the existence-row map and the discovered-path set, and warns once
/// per file. Both are needed: the map becomes the `files` rows, the set
/// feeds edge resolution. A file whose path has no UTF-8 representation is
/// left out of both — it has no usable key, and it is exactly the gap the
/// warn line exists to make visible.
///
/// A `not_utf8` skip keeps the Pass 1 content hash from `identities` as its
/// identity rather than a stat identity, so an edit that keeps the size and
/// mtime is still seen.
///
/// # Errors
///
/// [`PublishError::ChangedDuringSample`] when a skipped file was deleted
/// since the walk (see `sample::skipped_record`).
pub(super) fn collect_skipped(
    root: &Path,
    files: &[discovery::SkippedFile],
    identities: Option<&[SourceIdentity]>,
    discovered_paths: &mut HashSet<String>,
    records: &mut BTreeMap<String, SkippedRecord>,
) -> Result<(), PublishError> {
    let identity_by_path: HashMap<&str, &SourceIdentity> = identities
        .into_iter()
        .flatten()
        .map(|id| (id.relative_path.as_str(), id))
        .collect();
    for file in files {
        let Some(relative) = discovery::representable_relative_path(root, &file.absolute_path)
        else {
            warn_skipped(file);
            continue;
        };
        let mut record = sample::skipped_record(&relative, &file.absolute_path, file.code)?;
        if file.code == super::revision_skipped::SKIP_REASON_NOT_UTF8
            && let Some(id) = identity_by_path.get(relative.as_str())
        {
            // KEEP clone: record owns its SourceIdentity while identity_by_path borrows &SourceIdentity
            record.identity = (*id).clone();
        }
        // A failed stat is already warned about, with the path, the requested
        // reason and the source error, inside `skipped_record`; a second line
        // for the same file would break "every error is logged once".
        if record.reason != SKIP_REASON_STAT_FAILED {
            warn_skipped(file);
        }
        // KEEP clone: discovered_paths and records both own the relative path String
        discovered_paths.insert(relative.clone());
        records.insert(relative, record);
    }
    Ok(())
}

fn warn_skipped(file: &discovery::SkippedFile) {
    // `file.reason` is the skip message itself (for a sampling skip,
    // `SampleError::TooLarge`'s Display, which repeats the path). The path and
    // the static class identify the skip; the specific cause is carried by
    // the report.
    tracing::warn!(
        path = %file.absolute_path.display(),
        reason_kind = file.code,
        "skipped file during {}",
        file.code,
    );
}

/// Compares the current existence set — every sampled file *plus* every
/// discovered-but-skipped one — against the stored rows.
///
/// Both halves are the current set on purpose. A skipped file is on disk, so
/// leaving it out of the comparison would report it as `Deleted` on every
/// publish (its row would be re-deleted and re-inserted, and `PublishReport`
/// would claim deletions that never happened). Including it also makes the
/// manifest hash a statement about the project's files, not just the indexed
/// ones, so an unchanged tree still hashes to the same value.
///
/// Also flags classification or storage-shape drift (`file_kind` mismatch,
/// `indexed` row with `content IS NULL`, or non-`indexed` row with non-NULL
/// `content`) as `Modified` so a stale classification never persists.
pub(super) fn diff_against_stored(
    connection: &Connection,
    discovered: &[discovery::DiscoveredFile],
    identities: &[SourceIdentity],
    skipped: &[SkippedRecord],
    deadline: &Deadline,
) -> Result<Diff, PublishError> {
    check_deadline(deadline)?;
    let stored = stored_hashes(connection)?;
    let stored_refs: Vec<(&str, &str)> = stored
        .iter()
        .map(|row| (row.path.as_str(), row.content_hash.as_str()))
        .collect();
    let current_refs = current_hashes(identities, skipped);
    // KEEP clone: Vec<(&str, &str)> is borrowed again by load_failed, unchanged count, and aggregate_manifest_hash
    let mut changes = manifest::compare(stored_refs, current_refs.clone());
    add_classification_drift_changes(&mut changes, &stored, discovered, skipped);
    if crate::parse::fetch_gate::load_failed_retry_due()? {
        load_failed::add_load_failed_retries(
            &mut changes,
            &current_refs,
            &load_failed::load_failed_paths(connection)?,
        )?;
    }
    let added = count_changes(&changes, ChangeStatus::Added);
    let modified = count_changes(&changes, ChangeStatus::Modified);
    // Unchanged = current files with no Added or Modified change.
    let touched: HashSet<&str> = changes
        .iter()
        .filter(|change| change.status != ChangeStatus::Deleted)
        .map(|change| change.relative_path.as_str())
        .collect();
    let unchanged = current_refs
        .iter()
        .filter(|(path, _)| !touched.contains(path))
        .count();
    let manifest_hash = aggregate_manifest_hash(current_refs);
    Ok(Diff {
        deleted: count_changes(&changes, ChangeStatus::Deleted),
        unchanged,
        changes,
        manifest_hash,
        added,
        modified,
    })
}

/// Stored identity and classification fields read from `files` for diffing.
struct StoredRow {
    path: String,
    content_hash: String,
    file_kind: String,
    skip_reason: Option<String>,
    content_is_null: bool,
}

/// Marks any unchanged-hash row whose stored `file_kind`, `skip_reason`, or
/// `content` nullness contradicts the current discovery/skip classification as
/// `Modified`.
fn add_classification_drift_changes(
    changes: &mut Vec<FileChange>,
    stored: &[StoredRow],
    discovered: &[discovery::DiscoveredFile],
    skipped: &[SkippedRecord],
) {
    let already: HashSet<&str> = changes
        .iter()
        .map(|change| change.relative_path.as_str())
        .collect();
    let skipped_reasons: HashMap<&str, &str> = skipped
        .iter()
        .map(|record| (record.relative_path.as_str(), record.reason.as_str()))
        .collect();
    let discovered_kinds: HashMap<&str, discovery::FileKind> = discovered
        .iter()
        .map(|file| (file.relative_path.as_str(), file.kind))
        .collect();
    let mut drifted = Vec::new();
    for row in stored {
        let path = row.path.as_str();
        if already.contains(path) {
            continue;
        }
        let (expected_kind, expected_skip_reason) = if let Some(&reason) = skipped_reasons.get(path)
        {
            ("skipped", Some(reason))
        } else if let Some(&kind) = discovered_kinds.get(path) {
            let kind_str = match kind {
                discovery::FileKind::Indexed => "indexed",
                discovery::FileKind::Binary => "binary",
            };
            (kind_str, None)
        } else {
            continue;
        };
        if stored_row_drifted(row, expected_kind, expected_skip_reason) {
            drifted.push(FileChange {
                // KEEP clone: FileChange owns relative_path String while stored is borrowed
                relative_path: row.path.clone(),
                status: ChangeStatus::Modified,
            });
        }
    }
    if !drifted.is_empty() {
        changes.extend(drifted);
        changes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    }
}

/// Whether `row` disagrees with `expected_kind` (or `expected_skip_reason` for
/// skipped rows) or violates the `content` nullness invariant (`indexed` must
/// be non-NULL; non-`indexed` must be NULL).
///
/// A `skipped` row with `skip_reason = 'not_utf8'` is expected when Pass 1
/// discovered it as `indexed` (no NUL byte) and hashed the same non-UTF-8
/// bytes: Pass 1 does not UTF-8-validate, so matching hash + `not_utf8` is
/// steady state rather than drift.
fn stored_row_drifted(
    row: &StoredRow,
    expected_kind: &str,
    expected_skip_reason: Option<&str>,
) -> bool {
    let content_invalid = if row.file_kind == "indexed" {
        row.content_is_null
    } else {
        !row.content_is_null
    };
    if content_invalid {
        return true;
    }
    if row.file_kind == expected_kind {
        return expected_kind == "skipped" && row.skip_reason.as_deref() != expected_skip_reason;
    }
    !(row.file_kind == "skipped"
        && expected_kind == "indexed"
        && row.skip_reason.as_deref() == Some(super::revision_skipped::SKIP_REASON_NOT_UTF8))
}

/// Fails closed before the stored-row query when the shared budget is spent.
fn check_deadline(deadline: &Deadline) -> Result<(), PublishError> {
    match deadline.exceeded() {
        Some(elapsed) => Err(PublishError::TimeBudgetExceeded {
            path: String::new(),
            elapsed_ms: elapsed.as_millis(),
        }),
        None => Ok(()),
    }
}

fn stored_hashes(connection: &Connection) -> Result<Vec<StoredRow>, rusqlite::Error> {
    let mut stored_statement = connection
        .prepare("SELECT path, content_hash, file_kind, skip_reason, content IS NULL FROM files")?;
    let stored_rows = stored_statement
        .query_map([], |row| {
            Ok(StoredRow {
                path: row.get(0)?,
                content_hash: row.get(1)?,
                file_kind: row.get(2)?,
                skip_reason: row.get(3)?,
                content_is_null: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stored_rows)
}

/// The hashed and skipped files as `(path, content_hash)` pairs, in the
/// order `manifest::compare` and `aggregate_manifest_hash` consume.
fn current_hashes<'a>(
    identities: &'a [SourceIdentity],
    skipped: &'a [SkippedRecord],
) -> Vec<(&'a str, &'a str)> {
    let mut current_refs: Vec<(&str, &str)> = identities
        .iter()
        .map(|id| (id.relative_path.as_str(), id.content_hash.as_str()))
        .collect();
    current_refs.extend(skipped.iter().map(|record| {
        (
            record.relative_path.as_str(),
            record.identity.content_hash.as_str(),
        )
    }));
    current_refs
}

fn count_changes(changes: &[FileChange], status: ChangeStatus) -> usize {
    changes
        .iter()
        .filter(|change| change.status == status)
        .count()
}

pub(super) fn build_upserts_and_deletions(
    samples: Vec<Sample>,
    changes: &[FileChange],
    parser_version_changed: bool,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<(Vec<FileRecord>, Vec<String>), PublishError> {
    let changed_paths: Option<HashSet<&str>> = (!parser_version_changed).then(|| {
        changes
            .iter()
            .filter(|change| change.status != ChangeStatus::Deleted)
            .map(|change| change.relative_path.as_str())
            .collect()
    });
    let deletions: Vec<String> = changes
        .iter()
        .filter(|change| change.status == ChangeStatus::Deleted)
        // KEEP clone: deletions returns owned Vec<String> from borrowed &[FileChange]
        .map(|change| change.relative_path.clone())
        .collect();
    let capacity = changed_paths.as_ref().map_or(samples.len(), HashSet::len);
    let mut upserts = Vec::with_capacity(capacity);
    for sample in samples {
        let should_upsert = changed_paths
            .as_ref()
            .is_none_or(|paths| paths.contains(sample.relative_path.as_str()));
        if should_upsert {
            // `to_file_record` runs tree-sitter analysis — the publish
            // path's most expensive per-file work — so it must sit inside
            // the deadline check rather than after it.
            if let Some(elapsed) = deadline.exceeded() {
                return Err(PublishError::TimeBudgetExceeded {
                    path: format!(" while processing {}", sample.relative_path),
                    elapsed_ms: elapsed.as_millis(),
                });
            }
            upserts.push(to_file_record(sample, limits, deadline)?);
        }
    }
    Ok((upserts, deletions))
}

/// Retains only the skipped records that need to be written in this revision:
/// all of them when publishing from scratch or after a parser-version change,
/// otherwise only paths present in `changes` as added or modified.
pub(super) fn filter_skipped_records(
    skipped_records: Vec<SkippedRecord>,
    changes: &[FileChange],
    rewrite_all: bool,
) -> Vec<SkippedRecord> {
    if rewrite_all {
        return skipped_records;
    }
    let changed_paths: HashSet<&str> = changes
        .iter()
        .filter(|change| change.status != ChangeStatus::Deleted)
        .map(|change| change.relative_path.as_str())
        .collect();
    skipped_records
        .into_iter()
        .filter(|record| changed_paths.contains(record.relative_path.as_str()))
        .collect()
}

#[cfg(test)]
#[path = "publish_diff_tests.rs"]
mod tests;
