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
use super::revision_skipped::{
    SKIP_REASON_OVERSIZE, SKIP_REASON_STAT_FAILED, SKIP_REASON_UNREADABLE, SkippedRecord,
};
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

/// Everything one sampled-and-diffed attempt produces before the delta is
/// written: the inputs `publish_revision` needs, plus the report fields.
struct Attempt {
    discovered: Vec<discovery::DiscoveredFile>,
    discovered_paths: HashSet<String>,
    samples: Vec<sample::Sample>,
    diff: publish_diff::Diff,
    skipped: Vec<discovery::SkippedFile>,
    skipped_records: Vec<SkippedRecord>,
    /// Paths with no UTF-8 representation, persisted so `report` can count
    /// them without walking.
    unrepresentable: HashSet<Vec<u8>>,
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
    let unrepresentable = discovery::unrepresentable_paths(root, &discovery_skips);
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
        unrepresentable,
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

    let attempt = sample_and_diff(connection, root, sink, limits, deadline)?;
    // Persist what the walk could not represent before deciding whether the
    // revision itself changed: a non-UTF-8 add or remove with no
    // representable file change is a no-op revision but not a no-op count.
    super::publish::replace_unrepresentable_paths(connection, &attempt.unrepresentable)?;

    let parser_version_changed = baseline
        .as_ref()
        .is_none_or(|revision| revision.parser_pack_version != parser_pack_version);

    if attempt.diff.changes.is_empty()
        && !parser_version_changed
        && let Some(current) = baseline
    {
        return Ok(PublishReport {
            revision_id: current.revision_id,
            added: 0,
            modified: 0,
            deleted: 0,
            unchanged: attempt.diff.unchanged,
            skipped: attempt.skipped,
        });
    }

    commit_attempt(
        connection,
        expected_current,
        parser_pack_version,
        attempt,
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
        ..
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_capture::capture_warns;

    fn skipped(path: std::path::PathBuf) -> discovery::SkippedFile {
        discovery::SkippedFile {
            absolute_path: path,
            reason: "read failed".to_owned(),
        }
    }

    #[test]
    fn a_stat_failed_skip_logs_exactly_one_warn_line() {
        let root = tempfile::tempdir().expect("tempdir");
        let files = [skipped(root.path().join("vanished.rs"))];
        let mut paths = HashSet::new();
        let mut records = BTreeMap::new();

        let (_, logs) = capture_warns(|| {
            collect_skipped(
                root.path(),
                &files,
                "once-probe-stat-failed",
                &mut paths,
                &mut records,
            );
        });

        let lines = logs
            .lines()
            .filter(|line| line.contains("once-probe-stat-failed"))
            .count();
        assert_eq!(lines, 1, "one warn per stat-failed skip: {logs}");
        assert_eq!(records["vanished.rs"].reason, SKIP_REASON_STAT_FAILED);
        assert_eq!(records["vanished.rs"].byte_len, None);
    }

    #[test]
    fn a_skip_whose_stat_succeeds_keeps_the_publish_loop_warn() {
        let root = tempfile::tempdir().expect("tempdir");
        let present = root.path().join("present.rs");
        std::fs::write(&present, b"fn a() {}").expect("write file");
        let files = [skipped(present)];
        let mut paths = HashSet::new();
        let mut records = BTreeMap::new();

        let (_, logs) = capture_warns(|| {
            collect_skipped(
                root.path(),
                &files,
                "once-probe-stat-ok",
                &mut paths,
                &mut records,
            );
        });

        let lines = logs
            .lines()
            .filter(|line| line.contains("once-probe-stat-ok"))
            .count();
        assert_eq!(lines, 1, "{logs}");
        assert_eq!(records["present.rs"].reason, "once-probe-stat-ok");
    }
}
