// slugaudit-line-exception: approved-by=split-judge; reason=Pass 1 stored-row hash/drift diffing, skipped-file record filtering, and LoadFailed grammar retry detection
//! Diffs Pass 1 disk hashes and skip classifications against stored `files`
//! rows, including `LoadFailed` grammar retry detection.

use super::discovery;
use super::hash::aggregate_manifest_hash;
use super::manifest::{self, ChangeStatus, FileChange};
use super::publish::PublishError;
use super::revision::{SKIP_REASON_NOT_UTF8, SKIP_REASON_STAT_FAILED, SkippedRecord};
use super::sample;
use crate::model::SourceIdentity;
use crate::util::{Deadline, LockPoisoned};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub(super) struct Diff {
    pub(super) changes: Vec<FileChange>,
    pub(super) manifest_hash: String,
    pub(super) added: usize,
    pub(super) modified: usize,
    pub(super) deleted: usize,
    pub(super) unchanged: usize,
}

/// Folds skipped files into the existence-row map and `discovered_paths`,
/// warning once per file.
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
        if file.code == SKIP_REASON_NOT_UTF8
            && let Some(id) = identity_by_path.get(relative.as_str())
        {
            record.identity = (*id).clone();
        }
        if record.reason != SKIP_REASON_STAT_FAILED {
            warn_skipped(file);
        }
        discovered_paths.insert(relative.clone());
        records.insert(relative, record);
    }
    Ok(())
}

fn warn_skipped(file: &discovery::SkippedFile) {
    tracing::warn!(
        path = %file.absolute_path.display(),
        reason_kind = file.code,
        "skipped file during {}",
        file.code,
    );
}

/// Paths in `changes` whose status is `Added` or `Modified` (not `Deleted`).
pub(super) fn non_deleted_paths(changes: &[FileChange]) -> HashSet<&str> {
    changes
        .iter()
        .filter(|change| change.status != ChangeStatus::Deleted)
        .map(|change| change.relative_path.as_str())
        .collect()
}

/// Compares the current existence set (hashed files plus skipped files)
/// against stored `files` metadata headers without reading `files.content`.
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
    let mut changes = manifest::compare(stored_refs, current_refs.clone());
    add_classification_drift_changes(&mut changes, &stored, discovered, skipped);
    if crate::parse::fetch_gate::load_failed_retry_due()? {
        add_load_failed_retries(&mut changes, &current_refs, &load_failed_paths(connection)?)?;
    }
    let added = count_changes(&changes, ChangeStatus::Added);
    let modified = count_changes(&changes, ChangeStatus::Modified);
    let touched = non_deleted_paths(&changes);
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

/// Stored identity and classification header fields read from `files`.
struct StoredRow {
    path: String,
    content_hash: String,
    file_kind: String,
    skip_reason: Option<String>,
    content_is_null: bool,
}

fn extend_modified_changes(changes: &mut Vec<FileChange>, extra_paths: Vec<String>) {
    if extra_paths.is_empty() {
        return;
    }
    changes.extend(extra_paths.into_iter().map(|relative_path| FileChange {
        relative_path,
        status: ChangeStatus::Modified,
    }));
    changes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
}

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
            drifted.push(row.path.clone());
        }
    }
    extend_modified_changes(changes, drifted);
}

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
        && row.skip_reason.as_deref() == Some(SKIP_REASON_NOT_UTF8))
}

/// Marks still-present `LoadFailed` files as `Modified` once the grammar fetch
/// gate is open, skipping languages that suffered a permanent local load error.
pub(super) fn add_load_failed_retries(
    changes: &mut Vec<FileChange>,
    current: &[(&str, &str)],
    load_failed: &HashMap<String, Option<String>>,
) -> Result<(), LockPoisoned> {
    if load_failed.is_empty() {
        return Ok(());
    }
    let already: HashSet<&str> = changes
        .iter()
        .map(|change| change.relative_path.as_str())
        .collect();
    let mut retries = Vec::new();
    for (path, _) in current {
        let Some(language) = load_failed.get(*path) else {
            continue;
        };
        if already.contains(*path) {
            continue;
        }
        if let Some(lang) = language.as_deref()
            && crate::parse::local_load_failures::is_locally_failed(lang)?
        {
            continue;
        }
        retries.push((*path).to_owned());
    }
    extend_modified_changes(changes, retries);
    Ok(())
}

fn load_failed_paths(
    connection: &Connection,
) -> Result<HashMap<String, Option<String>>, rusqlite::Error> {
    let mut statement = connection
        .prepare("SELECT path, language FROM files WHERE parser_availability = 'LoadFailed'")?;
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect()
}

fn check_deadline(deadline: &Deadline) -> Result<(), PublishError> {
    match deadline.exceeded() {
        Some(elapsed) => Err(PublishError::TimeBudgetExceeded {
            activity: "diffing",
            path: None,
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

/// Retains only skipped records that need to be written in this revision.
pub(super) fn filter_skipped_records(
    skipped_records: Vec<SkippedRecord>,
    changes: &[FileChange],
    rewrite_all: bool,
) -> Vec<SkippedRecord> {
    if rewrite_all {
        return skipped_records;
    }
    let changed_paths = non_deleted_paths(changes);
    skipped_records
        .into_iter()
        .filter(|record| changed_paths.contains(record.relative_path.as_str()))
        .collect()
}

#[cfg(test)]
#[path = "publish_diff_tests.rs"]
mod tests;
