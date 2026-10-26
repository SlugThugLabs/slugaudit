//! Diffs freshly sampled disk state against what a revision currently has
//! stored, and turns that diff into the upsert/deletion sets a publish
//! writes. Kept separate from `publish` itself so that module stays focused
//! on orchestrating the sample → diff → revalidate → write sequence rather
//! than the mechanics of any one step.
use super::discovery;
use super::hash::aggregate_manifest_hash;
use super::manifest::{self, ChangeStatus, FileChange};
use super::publish::PublishError;
use super::revision::FileRecord;
use super::revision_skipped::{SKIP_REASON_STAT_FAILED, SkippedRecord};
use super::sample::{self, Sample, to_file_record};
use crate::model::ResourceLimits;
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

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
pub(super) fn collect_skipped(
    root: &Path,
    files: &[discovery::SkippedFile],
    reason: &str,
    discovered_paths: &mut HashSet<String>,
    records: &mut BTreeMap<String, SkippedRecord>,
) {
    for file in files {
        let Some(relative) = discovery::representable_relative_path(root, &file.absolute_path)
        else {
            warn_skipped(file, reason);
            continue;
        };
        let record = sample::skipped_record(&relative, &file.absolute_path, reason);
        // A failed stat is already warned about, with the path, the requested
        // reason and the source error, inside `skipped_record`; a second line
        // for the same file would break "every error is logged once".
        if record.reason != SKIP_REASON_STAT_FAILED {
            warn_skipped(file, reason);
        }
        discovered_paths.insert(relative.clone());
        records.insert(relative, record);
    }
}

fn warn_skipped(file: &discovery::SkippedFile, reason: &str) {
    // `file.reason` is the skip message itself: for a sampling skip it is
    // `SampleError::TooLarge`'s Display, which embeds the raw absolute path
    // and would defeat the redacted `path_id`. The static class is enough to
    // correlate the skip; the specific cause is carried by the report.
    tracing::warn!(
        path_id = %crate::util::redacted_path(&file.absolute_path),
        reason_kind = reason,
        "skipped file during {reason}"
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
pub(super) fn diff_against_stored(
    connection: &Connection,
    samples: &[Sample],
    skipped: &[SkippedRecord],
    deadline: &Deadline,
) -> Result<Diff, PublishError> {
    check_deadline(deadline)?;
    let stored = stored_hashes(connection)?;
    let stored_refs: Vec<(&str, &str)> = stored
        .iter()
        .map(|(path, hash)| (path.as_str(), hash.as_str()))
        .collect();
    let current_refs = current_hashes(samples, skipped);
    let mut changes = manifest::compare(stored_refs, current_refs.clone());
    if crate::parse::fetch_gate::load_failed_retry_due() {
        add_load_failed_retries(&mut changes, &current_refs, &load_failed_paths(connection)?);
    }
    let manifest_hash = aggregate_manifest_hash(current_refs);
    let added = count_changes(&changes, ChangeStatus::Added);
    let modified = count_changes(&changes, ChangeStatus::Modified);
    Ok(Diff {
        deleted: count_changes(&changes, ChangeStatus::Deleted),
        unchanged: samples.len() + skipped.len() - (added + modified),
        changes,
        manifest_hash,
        added,
        modified,
    })
}

/// Marks every still-present `LoadFailed` file as `Modified` so the publish
/// re-parses it even though its content hash is unchanged: its grammar could
/// not be fetched or loaded last time, and an unchanged hash would otherwise
/// keep that verdict forever. Only those rows are affected; the manifest hash
/// is computed from the current hashes and does not move.
fn add_load_failed_retries(
    changes: &mut Vec<FileChange>,
    current: &[(&str, &str)],
    load_failed: &HashSet<String>,
) {
    if load_failed.is_empty() {
        return;
    }
    let already: HashSet<String> = changes
        .iter()
        .map(|change| change.relative_path.clone())
        .collect();
    changes.extend(
        current
            .iter()
            .filter(|(path, _)| load_failed.contains(*path) && !already.contains(*path))
            .map(|(path, _)| FileChange {
                relative_path: (*path).to_owned(),
                status: ChangeStatus::Modified,
            }),
    );
    changes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
}

fn load_failed_paths(connection: &Connection) -> Result<HashSet<String>, rusqlite::Error> {
    let mut statement =
        connection.prepare("SELECT path FROM files WHERE parser_availability = 'LoadFailed'")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect()
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

fn stored_hashes(connection: &Connection) -> Result<Vec<(String, String)>, rusqlite::Error> {
    let mut stored_statement = connection.prepare("SELECT path, content_hash FROM files")?;
    let stored_rows = stored_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stored_rows)
}

/// The sampled and skipped files as `(path, content_hash)` pairs, in the
/// order `manifest::compare` and `aggregate_manifest_hash` consume.
fn current_hashes<'a>(
    samples: &'a [Sample],
    skipped: &'a [SkippedRecord],
) -> Vec<(&'a str, &'a str)> {
    let mut current_refs: Vec<(&str, &str)> = samples
        .iter()
        .map(|sample| {
            (
                sample.relative_path.as_str(),
                sample.identity.content_hash.as_str(),
            )
        })
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
    let changed_paths: HashSet<String> = if parser_version_changed {
        samples
            .iter()
            .map(|sample| sample.relative_path.clone())
            .collect()
    } else {
        changes
            .iter()
            .filter(|change| change.status != ChangeStatus::Deleted)
            .map(|change| change.relative_path.clone())
            .collect()
    };
    let deletions: Vec<String> = changes
        .iter()
        .filter(|change| change.status == ChangeStatus::Deleted)
        .map(|change| change.relative_path.clone())
        .collect();
    let mut upserts = Vec::with_capacity(changed_paths.len());
    for sample in samples {
        if changed_paths.contains(sample.relative_path.as_str()) {
            // `to_file_record` runs tree-sitter analysis — the publish
            // path's most expensive per-file work — so it must sit inside
            // the deadline check rather than after it.
            if let Some(elapsed) = deadline.exceeded() {
                return Err(PublishError::TimeBudgetExceeded {
                    path: format!(" while processing {}", sample.relative_path),
                    elapsed_ms: elapsed.as_millis(),
                });
            }
            upserts.push(to_file_record(sample, limits, deadline));
        }
    }
    Ok((upserts, deletions))
}
