//! A single publish attempt: discover, sample, diff, revalidate, and commit
//! one revision. The retry/CAS loop lives in `publish_cas` — this module is
//! pure orchestration of one attempt, with no retry logic of its own.
// slugaudit-line-exception: approved-by=slugthug; reason=one publish attempt end-to-end — skip collection and its warns, the sampling/diff half, the no-op short-circuit, and the CAS commit tail — reads as a single sequence; the Attempt struct and its two halves stay beside the ordering they exist to enforce

use super::discovery;
use super::publish::{PublishError, PublishReport};
use super::publish_diff::{self, build_upserts_and_deletions};
use super::race_hook;
use super::revalidate::revalidate_unchanged_since_sample;
use super::revision;
use super::revision_skipped::{SKIP_REASON_OVERSIZE, SKIP_REASON_UNREADABLE, SkippedRecord};
use super::sample::{self, sample_all_with_deadline};
use crate::model::ResourceLimits;
use crate::progress::ProgressSink;
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Folds one source of skipped files (discovery skips, then sampling skips)
/// into the existence-row map and the discovered-path set, and warns once
/// per file. Both are needed: the map becomes the `files` rows, the set
/// feeds edge resolution. A file whose path has no UTF-8 representation is
/// left out of both — it has no usable key, and it is exactly the gap the
/// warn line exists to make visible.
fn collect_skipped(
    root: &Path,
    files: &[discovery::SkippedFile],
    reason: &str,
    discovered_paths: &mut HashSet<String>,
    records: &mut BTreeMap<String, SkippedRecord>,
) {
    for file in files {
        tracing::warn!(
            path_id = %crate::util::redacted_path(&file.absolute_path),
            reason = %file.reason,
            "skipped file during {reason}"
        );
        let Some(relative) = discovery::representable_relative_path(root, &file.absolute_path)
        else {
            continue;
        };
        discovered_paths.insert(relative.clone());
        records.insert(
            relative.clone(),
            sample::skipped_record(&relative, &file.absolute_path, reason),
        );
    }
}

/// Everything one sampled-and-diffed attempt produces before the delta is
/// written: the inputs `publish_revision` needs, plus the report fields.
struct Attempt {
    discovered: Vec<discovery::DiscoveredFile>,
    discovered_paths: HashSet<String>,
    samples: Vec<sample::Sample>,
    diff: publish_diff::Diff,
    skipped: Vec<discovery::SkippedFile>,
    skipped_records: Vec<SkippedRecord>,
}

/// Discovery + sampling + skip accounting + diff: the read half of an
/// attempt, with every skip warned and recorded in one pass per class.
fn sample_and_diff(
    connection: &Connection,
    root: &Path,
    sink: &dyn ProgressSink,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<Attempt, PublishError> {
    let (discovered, discovery_skips) = discovery::discover_with_deadline(root, deadline)?;
    // Every path the walk saw on disk, including files sampling will skip
    // (oversize) — those exist, so an import targeting one must resolve
    // rather than be written as `Unresolved`.
    let mut discovered_paths: HashSet<String> = discovered
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    // Existence rows for the files we deliberately do not index, keyed by
    // path so one path can never be written twice and the order is
    // deterministic. A path with no UTF-8 representation, or one outside
    // the root, has no usable key — it stays visible only in
    // `PublishReport::skipped` and the log.
    let mut skipped_records: BTreeMap<String, SkippedRecord> = BTreeMap::new();
    collect_skipped(
        root,
        &discovery_skips,
        SKIP_REASON_UNREADABLE,
        &mut discovered_paths,
        &mut skipped_records,
    );
    let (samples, sampling_skips) = sample_all_with_deadline(&discovered, limits, sink, deadline)?;
    collect_skipped(
        root,
        &sampling_skips,
        SKIP_REASON_OVERSIZE,
        &mut discovered_paths,
        &mut skipped_records,
    );
    let skipped_records: Vec<SkippedRecord> = skipped_records.into_values().collect();
    let mut skipped = discovery_skips;
    skipped.extend(sampling_skips);
    race_hook::fire(root);
    let diff = publish_diff::diff_against_stored(connection, &samples, &skipped_records, deadline)?;
    Ok(Attempt {
        discovered,
        discovered_paths,
        samples,
        diff,
        skipped,
        skipped_records,
    })
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

    let Attempt {
        discovered,
        discovered_paths,
        samples,
        diff,
        skipped,
        skipped_records,
    } = sample_and_diff(connection, root, sink, limits, deadline)?;

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

    commit_attempt(
        connection,
        expected_current,
        parser_pack_version,
        Attempt {
            discovered,
            discovered_paths,
            samples,
            diff,
            skipped,
            skipped_records,
        },
        parser_version_changed,
        limits,
        deadline,
    )
}

/// The write half of an attempt: build the delta, revalidate what the diff
/// claimed was unchanged, and publish it as one CAS revision.
fn commit_attempt(
    connection: &mut Connection,
    expected_current: Option<&str>,
    parser_pack_version: &str,
    attempt: Attempt,
    parser_version_changed: bool,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<PublishReport, PublishError> {
    let Attempt {
        discovered,
        discovered_paths,
        samples,
        diff,
        skipped,
        skipped_records,
    } = attempt;
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
        revision::RevisionInputs {
            expected_current,
            manifest_hash: &diff.manifest_hash,
            parser_pack_version,
            upserts: &upserts,
            deletions: &deletions,
            skipped: &skipped_records,
            discovered_paths: &discovered_paths,
        },
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
