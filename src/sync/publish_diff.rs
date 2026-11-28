// slugaudit-line-exception: approved-by=split-judge; reason=Pass 1 stored-row hash/drift diffing, skipped-file record filtering, and LoadFailed grammar retry detection
//! Diffs Pass 1 disk hashes and skip classifications against stored `files`
//! rows, including `LoadFailed` grammar retry detection.

use super::discovery;
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
    let load_failed_due = crate::parse::fetch_gate::load_failed_retry_due()?;
    let stored_state = load_stored_state(connection, load_failed_due)?;
    let stored_refs: Vec<(&str, &str)> = stored_state
        .stored
        .iter()
        .map(|row| (row.path.as_str(), row.content_hash.as_str()))
        .collect();
    let current_refs = current_hashes(identities, skipped);
    let mut changes = manifest::compare(stored_refs, current_refs.clone());
    add_classification_drift_changes(&mut changes, &stored_state.stored, discovered, skipped);
    add_available_grammar_retries(&mut changes, &current_refs, &stored_state.unavailable)?;
    if load_failed_due {
        add_load_failed_retries(&mut changes, &current_refs, &stored_state.load_failed)?;
    }
    let added = count_changes(&changes, ChangeStatus::Added);
    let modified = count_changes(&changes, ChangeStatus::Modified);
    let touched = non_deleted_paths(&changes);
    let unchanged = current_refs
        .iter()
        .filter(|(path, _)| !touched.contains(path))
        .count();
    Ok(Diff {
        deleted: count_changes(&changes, ChangeStatus::Deleted),
        unchanged,
        changes,
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

fn language_retry_ready<'a>(
    lang: &'a str,
    retry_due: bool,
    require_available: bool,
    ready_by_lang: &mut HashMap<&'a str, bool>,
) -> Result<bool, LockPoisoned> {
    if let Some(&ready) = ready_by_lang.get(lang) {
        return Ok(ready);
    }
    let ready = (!require_available || crate::parse::language_available(lang))
        && (retry_due || crate::parse::fetch_gate::is_grammar_installed(lang)?)
        && !crate::parse::local_load_failures::is_locally_failed(lang)?;
    ready_by_lang.insert(lang, ready);
    Ok(ready)
}

fn collect_retry_paths<'a, V>(
    changes: &mut Vec<FileChange>,
    current: &[(&str, &str)],
    candidates: &'a HashMap<String, V>,
    retry_due: bool,
    require_available: bool,
    lang_of: impl Fn(&'a V) -> Option<&'a str>,
) -> Result<(), LockPoisoned> {
    let already: HashSet<&str> = changes
        .iter()
        .map(|change| change.relative_path.as_str())
        .collect();
    let mut ready_by_lang: HashMap<&'a str, bool> = HashMap::new();
    let mut retries = Vec::new();
    for (path, _) in current {
        let Some(value) = candidates.get(*path) else {
            continue;
        };
        if already.contains(*path) {
            continue;
        }
        if let Some(lang) = lang_of(value)
            && !language_retry_ready(lang, retry_due, require_available, &mut ready_by_lang)?
        {
            continue;
        }
        retries.push((*path).to_owned());
    }
    extend_modified_changes(changes, retries);
    Ok(())
}

/// Marks still-present `Unavailable` files as `Modified` when their language is
/// known to the pack, ready to fetch or already installed, and not locally
/// failed.
pub(crate) fn add_available_grammar_retries(
    changes: &mut Vec<FileChange>,
    current: &[(&str, &str)],
    unavailable: &HashMap<String, String>,
) -> Result<(), LockPoisoned> {
    if unavailable.is_empty() {
        return Ok(());
    }
    let retry_due = crate::parse::fetch_gate::load_failed_retry_due()?;
    collect_retry_paths(changes, current, unavailable, retry_due, true, |lang| {
        Some(lang.as_str())
    })
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
    let retry_due = crate::parse::fetch_gate::load_failed_retry_due()?;
    collect_retry_paths(
        changes,
        current,
        load_failed,
        retry_due,
        false,
        Option::as_deref,
    )
}

struct StoredState {
    stored: Vec<StoredRow>,
    unavailable: HashMap<String, String>,
    load_failed: HashMap<String, Option<String>>,
}

fn load_stored_state(
    connection: &Connection,
    include_load_failed: bool,
) -> Result<StoredState, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT path, content_hash, file_kind, skip_reason, content IS NULL, \
                CASE WHEN (file_kind = 'indexed' AND parser_availability = 'Unavailable' \
                           AND parse_outcome = 'NotAttempted' AND parse_error_reason IS NULL \
                           AND language IS NOT NULL AND language <> 'prose') \
                       OR (?1 AND parser_availability = 'LoadFailed') \
                     THEN language ELSE NULL END, \
                CASE WHEN file_kind = 'indexed' AND parser_availability = 'Unavailable' \
                          AND parse_outcome = 'NotAttempted' AND parse_error_reason IS NULL \
                          AND language IS NOT NULL AND language <> 'prose' \
                     THEN 1 \
                     WHEN ?1 AND parser_availability = 'LoadFailed' \
                     THEN 2 \
                     ELSE 0 END \
         FROM files",
    )?;
    let mut rows = statement.query([include_load_failed])?;
    let mut stored = Vec::new();
    let mut unavailable = HashMap::new();
    let mut load_failed = HashMap::new();
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        let content_hash: String = row.get(1)?;
        let file_kind: String = row.get(2)?;
        let skip_reason: Option<String> = row.get(3)?;
        let content_is_null: bool = row.get(4)?;
        let retry_lang: Option<String> = row.get(5)?;
        let retry_kind: i64 = row.get(6)?;

        match retry_kind {
            1 => {
                if let Some(lang) = retry_lang {
                    unavailable.insert(path.clone(), lang);
                }
            }
            2 => {
                load_failed.insert(path.clone(), retry_lang);
            }
            _ => {}
        }

        stored.push(StoredRow {
            path,
            content_hash,
            file_kind,
            skip_reason,
            content_is_null,
        });
    }
    Ok(StoredState {
        stored,
        unavailable,
        load_failed,
    })
}

#[cfg(test)]
fn unavailable_language_paths(
    connection: &Connection,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    Ok(load_stored_state(connection, false)?.unavailable)
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
