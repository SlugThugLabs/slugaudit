//! Diffs freshly sampled disk state against what a revision currently has
//! stored, and turns that diff into the upsert/deletion sets a publish
//! writes. Kept separate from `publish` itself so that module stays focused
//! on orchestrating the sample → diff → revalidate → write sequence rather
//! than the mechanics of any one step.
use super::hash::aggregate_manifest_hash;
use super::manifest::{self, ChangeStatus, FileChange};
use super::publish::PublishError;
use super::revision::FileRecord;
use super::revision_skipped::SkippedRecord;
use super::sample::{Sample, to_file_record};
use crate::model::ResourceLimits;
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::HashSet;

pub(super) struct Diff {
    pub(super) changes: Vec<FileChange>,
    pub(super) manifest_hash: String,
    pub(super) added: usize,
    pub(super) modified: usize,
    pub(super) deleted: usize,
    pub(super) unchanged: usize,
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
    if let Some(elapsed) = deadline.exceeded() {
        return Err(PublishError::TimeBudgetExceeded {
            path: String::new(),
            elapsed_ms: elapsed.as_millis(),
        });
    }
    let mut stored_statement = connection.prepare("SELECT path, content_hash FROM files")?;
    let stored_rows = stored_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stored_statement);

    let stored_refs: Vec<(&str, &str)> = stored_rows
        .iter()
        .map(|(path, hash)| (path.as_str(), hash.as_str()))
        .collect();
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
    let changes = manifest::compare(stored_refs, current_refs.clone());
    let manifest_hash = aggregate_manifest_hash(current_refs);

    let added = changes
        .iter()
        .filter(|c| c.status == ChangeStatus::Added)
        .count();
    let modified = changes
        .iter()
        .filter(|c| c.status == ChangeStatus::Modified)
        .count();
    let deleted = changes
        .iter()
        .filter(|c| c.status == ChangeStatus::Deleted)
        .count();
    let unchanged = samples.len() + skipped.len() - (added + modified);

    Ok(Diff {
        changes,
        manifest_hash,
        added,
        modified,
        deleted,
        unchanged,
    })
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
            upserts.push(to_file_record(sample, limits));
        }
    }
    Ok((upserts, deletions))
}
