//! A single publish attempt: discover, sample, diff, revalidate, and commit
//! one revision. The retry/CAS loop lives in `publish_cas` — this module is
//! pure orchestration of one attempt, with no retry logic of its own.

use super::discovery;
use super::publish::{PublishError, PublishReport};
use super::publish_diff::{build_upserts_and_deletions, diff_against_stored};
use super::race_hook;
use super::revalidate::revalidate_unchanged_since_sample;
use super::revision;
use super::sample::sample_all_with_deadline;
use crate::model::ResourceLimits;
use crate::progress::ProgressSink;
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

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

/// One publish attempt: discovers, hashes, and diffs the project's current
/// disk state against what the database has stored, then publishes exactly
/// the delta as one atomic CAS revision. A disk state identical to the
/// current revision skips publishing entirely rather than churning out a
/// no-op revision.
///
/// The caller (`publish_cas::publish`) wraps this in a retry loop for the
/// two retryable hazards: another publisher winning the CAS race, and a
/// file changing on disk after being sampled.
///
/// # Errors
///
/// Returns an error if discovery, sampling, diffing, revalidation, or the
/// revision publish fails. Retryable errors are `StaleBaseline` (another
/// publisher won) and `ChangedDuringSample` (a file changed mid-sample).
pub(super) fn try_publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
    sink: &dyn ProgressSink,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<PublishReport, PublishError> {
    let baseline = current_revision(connection)?;
    let expected_current = baseline
        .as_ref()
        .map(|revision| revision.revision_id.as_str());

    let (discovered, skipped) = discovery::discover_with_deadline(root, deadline)?;
    // Every path the walk saw on disk, including files sampling will skip
    // (oversize) — those exist, so an import targeting one must resolve
    // rather than be written as `Unresolved`. They never get a `files`
    // row, so edge resolution cannot learn about them from the database.
    let mut discovered_paths: HashSet<String> = discovered
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    // Discovery also skipped files it walked but could not read (mode
    // 000, a filesystem error): they exist too. A path with no UTF-8
    // representation, or one outside the root, has no usable key and is
    // deliberately left out — the same reason discovery could not index
    // it. `replace` mirrors discovery's own relative-path normalization.
    for file in &skipped {
        if let Ok(relative) = file.absolute_path.strip_prefix(root)
            && let Some(relative) = relative.to_str()
            && !relative.is_empty()
        {
            discovered_paths.insert(relative.replace('\\', "/"));
        }
    }
    if !skipped.is_empty() {
        for file in &skipped {
            tracing::warn!(
                path = %file.absolute_path.display(),
                reason = %file.reason,
                "skipping unreadable/invalid file during discovery"
            );
        }
    }
    let (samples, sampling_skips) = sample_all_with_deadline(&discovered, limits, sink, deadline)?;
    if !sampling_skips.is_empty() {
        for file in &sampling_skips {
            tracing::warn!(
                path = %file.absolute_path.display(),
                reason = %file.reason,
                "skipping oversized file during sampling"
            );
        }
    }
    let mut skipped = skipped;
    skipped.extend(sampling_skips);
    race_hook::fire(root);
    let diff = diff_against_stored(connection, &samples, discovered.len(), deadline)?;

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
            skipped,
        });
    }

    let (upserts, deletions) = build_upserts_and_deletions(
        samples,
        &diff.changes,
        parser_version_changed,
        limits,
        deadline,
    )?;
    revalidate_unchanged_since_sample(&discovered, &upserts, limits, deadline)?;

    let revision_id = revision::publish_revision(
        connection,
        expected_current,
        &diff.manifest_hash,
        parser_pack_version,
        &upserts,
        &deletions,
        &discovered_paths,
    )?;

    Ok(PublishReport {
        revision_id,
        added: diff.added,
        modified: diff.modified,
        deleted: diff.deleted,
        unchanged: diff.unchanged,
        skipped,
    })
}
