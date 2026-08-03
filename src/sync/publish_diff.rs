//! Diffs freshly sampled disk state against what a revision currently has
//! stored, and turns that diff into the upsert/deletion sets a publish
//! writes. Kept separate from `publish` itself so that module stays focused
//! on orchestrating the sample → diff → revalidate → write sequence rather
//! than the mechanics of any one step.
use super::hash::aggregate_manifest_hash;
use super::manifest::{self, ChangeStatus, FileChange};
use super::publish::PublishError;
use super::revision::FileRecord;
use super::sample::{Sample, to_file_record};
use crate::model::ResourceLimits;
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

pub(super) fn diff_against_stored(
    connection: &Connection,
    samples: &[Sample],
    discovered_len: usize,
) -> Result<Diff, PublishError> {
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
    let current_refs: Vec<(&str, &str)> = samples
        .iter()
        .map(|sample| {
            (
                sample.relative_path.as_str(),
                sample.identity.content_hash.as_str(),
            )
        })
        .collect();
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
    let unchanged = discovered_len - (added + modified);

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
) -> (Vec<FileRecord>, Vec<String>) {
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
    let upserts: Vec<FileRecord> = samples
        .into_iter()
        .filter(|sample| changed_paths.contains(sample.relative_path.as_str()))
        .map(|sample| to_file_record(sample, limits))
        .collect();
    (upserts, deletions)
}
