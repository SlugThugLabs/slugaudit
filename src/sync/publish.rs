use super::discovery::{self, DiscoveredFile};
use super::publish_diff::{build_upserts_and_deletions, diff_against_stored};
use super::race_hook;
use super::revision::{self, FileRecord, RevisionError};
use super::sample::{self, Sample, sample_file};
use crate::model::ResourceLimits;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// How many times a retryable publish failure (a concurrent publisher won
/// the CAS race, or a file changed on disk after being sampled) is retried
/// before the error surfaces to the tool caller. Each retry re-discovers
/// and re-samples the filesystem from scratch — it never reuses a stale
/// sample.
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
    /// A file's on-disk content changed between being sampled and the
    /// publish transaction that would have written it as current. Caught by
    /// `revalidate_unchanged_since_sample` immediately before the write;
    /// the caller retries with an entirely fresh sample rather than
    /// publishing a revision built from stale bytes.
    #[error("{path} changed on disk after being sampled; retrying with a fresh sample")]
    ChangedDuringSample { path: String },
}

fn is_retryable(error: &PublishError) -> bool {
    matches!(
        error,
        PublishError::Revision(RevisionError::StaleBaseline { .. })
            | PublishError::ChangedDuringSample { .. }
    )
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
///
/// Two independent hazards are retried the same way, up to
/// `MAX_CAS_RETRIES` times, each retry re-sampling from scratch: another
/// publisher committing first (compare-and-swap on the current revision
/// id), and a file changing on disk after this attempt sampled it but
/// before the write transaction it would land in. Neither is silently
/// papered over — both fail closed into a retry rather than publishing a
/// revision that doesn't match a real, stable disk state.
///
/// # Errors
///
/// Returns an error if discovery, reading a discovered file, resource
/// limits are exceeded, or any database operation in the publish
/// transaction fails (including exhausting retries).
pub fn publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
) -> Result<PublishReport, PublishError> {
    let mut attempt = 0_usize;
    loop {
        let result = try_publish(connection, root, parser_pack_version);
        match &result {
            Err(error) if is_retryable(error) && attempt + 1 < MAX_CAS_RETRIES => {
                attempt += 1;
            }
            _ => return result,
        }
    }
}

fn sample_all(
    discovered: &[DiscoveredFile],
    limits: &ResourceLimits,
) -> Result<Vec<Sample>, PublishError> {
    let mut total_bytes = 0_u64;
    let mut samples = Vec::with_capacity(discovered.len());
    for file in discovered {
        let sample = sample_file(file, limits)?;
        total_bytes = total_bytes.saturating_add(sample.byte_len);
        if total_bytes > limits.max_total_import_bytes {
            return Err(PublishError::ImportTooLarge {
                total: total_bytes,
                limit: limits.max_total_import_bytes,
            });
        }
        samples.push(sample);
    }
    Ok(samples)
}

/// A revision published by this crate is not a true point-in-time atomic
/// snapshot of the filesystem — no OS-level snapshot is taken, and nothing
/// locks files against concurrent editors. It is instead a *verified
/// collection of individually stable files*: every file this function is
/// about to write is re-sampled and re-hashed one last time, and the
/// publish is aborted (to be retried with an entirely fresh sample) if any
/// of them no longer matches what was recorded during the original sample.
/// This closes the gap between "we read this file" and "we told the
/// database this is the file's current state" as tightly as is achievable
/// without filesystem-level locking, which ordinary editors (atomic
/// rename-on-save in particular) would defeat anyway.
fn revalidate_unchanged_since_sample(
    discovered: &[DiscoveredFile],
    upserts: &[FileRecord],
    limits: &ResourceLimits,
) -> Result<(), PublishError> {
    let by_path: HashMap<&str, &DiscoveredFile> = discovered
        .iter()
        .map(|file| (file.relative_path.as_str(), file))
        .collect();
    for upsert in upserts {
        let unchanged = by_path
            .get(upsert.relative_path.as_str())
            .is_some_and(|file| {
                sample_file(file, limits)
                    .is_ok_and(|fresh| fresh.identity.content_hash == upsert.identity.content_hash)
            });
        if !unchanged {
            return Err(PublishError::ChangedDuringSample {
                path: upsert.relative_path.clone(),
            });
        }
    }
    Ok(())
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
    let samples = sample_all(&discovered, &limits)?;
    race_hook::fire(root);
    let diff = diff_against_stored(connection, &samples, discovered.len())?;

    // A file's content not changing doesn't mean its evidence is still
    // valid — if the parser itself was upgraded, everything needs
    // re-analysis even though every hash still matches.
    let parser_version_changed = baseline
        .as_ref()
        .is_none_or(|revision| revision.parser_pack_version != parser_pack_version);

    if diff.changes.is_empty()
        && !parser_version_changed
        && let Some(current) = baseline
    {
        return Ok(PublishReport {
            revision_id: current.revision_id,
            added: 0,
            modified: 0,
            deleted: 0,
            unchanged: diff.unchanged,
        });
    }

    let (upserts, deletions) =
        build_upserts_and_deletions(samples, &diff.changes, parser_version_changed, &limits);
    revalidate_unchanged_since_sample(&discovered, &upserts, &limits)?;

    let revision_id = revision::publish_revision(
        connection,
        expected_current,
        &diff.manifest_hash,
        parser_pack_version,
        &upserts,
        &deletions,
    )?;

    Ok(PublishReport {
        revision_id,
        added: diff.added,
        modified: diff.modified,
        deleted: diff.deleted,
        unchanged: diff.unchanged,
    })
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "publish_race_tests.rs"]
mod race_tests;
