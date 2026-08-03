use super::discovery;
use super::hash::aggregate_manifest_hash;
use super::manifest::{self, ChangeStatus};
use super::revision::{self, FileRecord, RevisionError};
use super::sample::{self, sample_file, to_file_record};
use crate::model::ResourceLimits;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;
use thiserror::Error;

/// How many times a concurrent-publish CAS failure is retried before the
/// error surfaces to the tool caller. Each retry re-samples the filesystem.
const MAX_CAS_RETRIES: usize = 4;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error(transparent)]
    Discovery(#[from] discovery::DiscoveryError),
    #[error(transparent)]
    Sample(#[from] sample::SampleError),
    #[error(transparent)]
    Revision(#[from] revision::RevisionError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "import would load {total} bytes across sampled files, exceeding the {limit}-byte ceiling"
    )]
    ImportTooLarge { total: u64, limit: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub revision_id: String,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub unchanged: usize,
}

struct CurrentRevision {
    revision_id: String,
    parser_pack_version: String,
}

fn current_revision(connection: &Connection) -> Result<Option<CurrentRevision>, PublishError> {
    connection
        .query_row(
            "SELECT revision_id, parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| {
                Ok(CurrentRevision {
                    revision_id: row.get(0)?,
                    parser_pack_version: row.get(1)?,
                })
            },
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(PublishError::Database(other)),
        })
}

/// Discovers, hashes, and diffs the project's current disk state against
/// what the database has stored, then publishes exactly the delta as one
/// atomic CAS revision. A disk state identical to the current revision
/// skips publishing entirely rather than churning out a no-op revision.
/// Concurrent publishers are serialized by compare-and-swap on the current
/// revision id: a loser re-samples and retries a bounded number of times.
///
/// # Errors
///
/// Returns an error if discovery, reading a discovered file, resource
/// limits are exceeded, or any database operation in the publish
/// transaction fails (including exhausting CAS retries).
pub fn publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
) -> Result<PublishReport, PublishError> {
    let mut last_stale = None;
    for _ in 0..MAX_CAS_RETRIES {
        match try_publish(connection, root, parser_pack_version) {
            Err(PublishError::Revision(RevisionError::StaleBaseline { expected, found })) => {
                last_stale = Some(RevisionError::StaleBaseline { expected, found });
            }
            other => return other,
        }
    }
    Err(PublishError::Revision(last_stale.expect(
        "CAS loop only continues after StaleBaseline; retries exhausted implies one was stored",
    )))
}

fn try_publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
) -> Result<PublishReport, PublishError> {
    // Baseline is read before the expensive filesystem sample so the write
    // transaction can refuse to commit if another publisher landed first.
    let baseline = current_revision(connection)?;
    let expected_current = baseline
        .as_ref()
        .map(|revision| revision.revision_id.as_str());

    let discovered = discovery::discover(root)?;
    let limits = ResourceLimits::default();
    let mut total_bytes = 0_u64;
    let mut samples = Vec::with_capacity(discovered.len());
    for file in &discovered {
        let sample = sample_file(file, &limits)?;
        total_bytes = total_bytes.saturating_add(sample.byte_len);
        if total_bytes > limits.max_total_import_bytes {
            return Err(PublishError::ImportTooLarge {
                total: total_bytes,
                limit: limits.max_total_import_bytes,
            });
        }
        samples.push(sample);
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
    let unchanged = discovered.len() - (added + modified);

    // A file's content not changing doesn't mean its evidence is still
    // valid — if the parser itself was upgraded, everything needs
    // re-analysis even though every hash still matches.
    let parser_version_changed = baseline
        .as_ref()
        .is_none_or(|revision| revision.parser_pack_version != parser_pack_version);

    if changes.is_empty()
        && !parser_version_changed
        && let Some(current) = baseline
    {
        return Ok(PublishReport {
            revision_id: current.revision_id,
            added: 0,
            modified: 0,
            deleted: 0,
            unchanged,
        });
    }

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
        .map(|sample| to_file_record(sample, &limits))
        .collect();

    let revision_id = revision::publish_revision(
        connection,
        expected_current,
        &manifest_hash,
        parser_pack_version,
        &upserts,
        &deletions,
    )?;

    Ok(PublishReport {
        revision_id,
        added,
        modified,
        deleted,
        unchanged,
    })
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
