// slugaudit-line-exception: approved-by=slugthuglabs; reason=one publish attempt: two-pass streaming discovery, hash, diff, selective sample, and atomic commit
//! A single publish attempt: discover, stream-hash, diff, sample changed files, and commit
//! one revision. The retry/CAS loop lives in `publish_cas` — this module is
//! pure orchestration of one attempt, with no retry logic of its own.

use super::discovery;
use super::hash_batch;
use super::manifest::{ChangeStatus, FileChange};
use super::publish::{PublishError, PublishReport};
use super::publish_diff::{self, build_upserts_and_deletions};
use super::publish_sample::{check_samples_match_pass1, files_to_sample};
use super::race_hook::{self, RacePoint};
use super::revalidate::revalidate_unchanged_since_sample;
use super::revision;
use super::revision_skipped::SkippedRecord;
use super::sample::{self, sample_all_with_deadline};
use crate::model::{ResourceLimits, SourceIdentity};
use crate::progress::ProgressSink;
use crate::util::Deadline;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Everything Pass 1 (discovery + stream hashing + diff) produces.
struct DiffState {
    discovered: Vec<discovery::DiscoveredFile>,
    discovered_paths: HashSet<String>,
    identities: Vec<SourceIdentity>,
    diff: publish_diff::Diff,
    skipped: Vec<discovery::SkippedFile>,
    skipped_records: BTreeMap<String, SkippedRecord>,
    unrepresentable: HashSet<Vec<u8>>,
}

/// Pass 1: Discover and stream-hash all files on disk, then diff against stored hashes.
/// No file text is allocated or held in memory during this pass.
fn hash_and_diff(
    connection: &Connection,
    root: &Path,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<DiffState, PublishError> {
    let (discovered, discovery_skips) = discovery::discover_with_deadline(root, deadline)?;
    race_hook::fire(root, RacePoint::AfterDiscovery);
    let unrepresentable = discovery::unrepresentable_paths(root, &discovery_skips);
    let mut discovered_paths: HashSet<String> = discovered
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    let mut skipped_records: BTreeMap<String, SkippedRecord> = BTreeMap::new();
    publish_diff::collect_skipped(
        root,
        &discovery_skips,
        None,
        &mut discovered_paths,
        &mut skipped_records,
    )?;

    let (identities, hashing_skips) =
        hash_batch::hash_all_with_deadline(&discovered, limits.max_file_bytes, deadline)?;
    publish_diff::collect_skipped(
        root,
        &hashing_skips,
        None,
        &mut discovered_paths,
        &mut skipped_records,
    )?;
    let skipped_records_vec: Vec<SkippedRecord> = skipped_records.values().cloned().collect();
    let mut skipped = discovery_skips;
    skipped.extend(hashing_skips);

    let mut diff =
        publish_diff::diff_against_stored(connection, &identities, &skipped_records_vec, deadline)?;
    let mismatched = super::verify_stored::content_mismatches(connection, deadline)?;
    force_content_repairs(&mut diff, &mismatched, &identities);

    Ok(DiffState {
        discovered,
        discovered_paths,
        identities,
        diff,
        skipped,
        skipped_records,
        unrepresentable,
    })
}

/// Marks every database-side content mismatch that still exists on disk as
/// `Modified`, so the publish re-reads the file and rewrites the row.
fn force_content_repairs(
    diff: &mut publish_diff::Diff,
    mismatched: &HashSet<String>,
    identities: &[SourceIdentity],
) {
    if mismatched.is_empty() {
        return;
    }
    let present: HashSet<&str> = identities
        .iter()
        .map(|id| id.relative_path.as_str())
        .collect();
    let mut already: HashSet<String> = diff
        .changes
        .iter()
        .map(|change| change.relative_path.clone())
        .collect();
    for path in mismatched {
        if !present.contains(path.as_str()) || !already.insert(path.clone()) {
            continue;
        }
        diff.modified += 1;
        diff.unchanged = diff.unchanged.saturating_sub(1);
        diff.changes.push(FileChange {
            relative_path: path.clone(),
            status: ChangeStatus::Modified,
        });
    }
    diff.changes
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
}

struct CurrentRevision {
    revision_id: String,
    parser_pack_version: String,
}

fn current_revision(connection: &Connection) -> Result<Option<CurrentRevision>, PublishError> {
    let row = revision::current_revision_row(connection).map_err(PublishError::Database)?;
    Ok(
        row.map(|(revision_id, parser_pack_version)| CurrentRevision {
            revision_id,
            parser_pack_version,
        }),
    )
}

/// The no-change fast path: re-reads the current revision after the race
/// hook, fails with `StaleBaseline` if another writer moved it, and otherwise
/// reports the baseline revision with zero changes.
fn unchanged_report(
    connection: &Connection,
    root: &Path,
    baseline: &CurrentRevision,
    unchanged: usize,
    skipped: Vec<discovery::SkippedFile>,
) -> Result<PublishReport, PublishError> {
    race_hook::fire(root, RacePoint::BeforeCommit);
    let current_now = current_revision(connection)?;
    let current_id_now = current_now.as_ref().map(|r| r.revision_id.as_str());
    if current_id_now != Some(baseline.revision_id.as_str()) {
        return Err(PublishError::Revision(
            revision::RevisionError::StaleBaseline {
                expected: baseline.revision_id.clone(),
                found: revision::baseline_label(current_id_now),
            },
        ));
    }
    Ok(PublishReport {
        revision_id: baseline.revision_id.clone(),
        added: 0,
        modified: 0,
        deleted: 0,
        unchanged,
        skipped,
    })
}

/// One publish attempt: discovers, stream-hashes, and diffs the project's current
/// disk state against what the database has stored. If unchanged, finishes immediately
/// without reading file contents. If changed, samples only modified files and publishes.
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

    let state = hash_and_diff(connection, root, limits, deadline)?;
    super::publish::replace_unrepresentable_paths(connection, &state.unrepresentable)?;

    let parser_version_changed = baseline
        .as_ref()
        .is_none_or(|revision| revision.parser_pack_version != parser_pack_version);

    if state.diff.changes.is_empty()
        && !parser_version_changed
        && let Some(current) = &baseline
    {
        return unchanged_report(
            connection,
            root,
            current,
            state.diff.unchanged,
            state.skipped,
        );
    }

    let files_to_sample = files_to_sample(
        &state.discovered,
        &state.identities,
        &state.diff.changes,
        parser_version_changed,
    );
    let (samples, sampling_skips) =
        sample_all_with_deadline(&files_to_sample, limits, sink, deadline)?;
    check_samples_match_pass1(&samples, &state.identities)?;

    let mut discovered_paths = state.discovered_paths;
    let mut skipped_records = state.skipped_records;
    publish_diff::collect_skipped(
        root,
        &sampling_skips,
        Some(&state.identities),
        &mut discovered_paths,
        &mut skipped_records,
    )?;
    let mut skipped = state.skipped;
    skipped.extend(sampling_skips);
    let skipped_records: Vec<SkippedRecord> = skipped_records.into_values().collect();
    race_hook::fire(root, RacePoint::BeforeCommit);

    let inputs = CommitInputs {
        discovered: &state.discovered,
        discovered_paths,
        samples,
        diff: state.diff,
        skipped,
        skipped_records,
    };

    commit_attempt(
        connection,
        expected_current,
        parser_pack_version,
        inputs,
        parser_version_changed,
        limits,
        deadline,
    )
}

/// The write-side inputs prepared across Pass 1 and Pass 2.
struct CommitInputs<'a> {
    discovered: &'a [discovery::DiscoveredFile],
    discovered_paths: HashSet<String>,
    samples: Vec<sample::Sample>,
    diff: publish_diff::Diff,
    skipped: Vec<discovery::SkippedFile>,
    skipped_records: Vec<SkippedRecord>,
}

/// The write half of an attempt: build the delta, revalidate what the diff
/// claimed was unchanged, and publish it as one CAS revision.
fn commit_attempt(
    connection: &mut Connection,
    expected_current: Option<&str>,
    parser_pack_version: &str,
    inputs: CommitInputs<'_>,
    parser_version_changed: bool,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<PublishReport, PublishError> {
    let CommitInputs {
        discovered,
        discovered_paths,
        samples,
        diff,
        skipped,
        skipped_records,
    } = inputs;
    let re_resolve_unresolved = diff.added > 0 || diff.deleted > 0;
    let (upserts, deletions) = build_upserts_and_deletions(
        samples,
        &diff.changes,
        parser_version_changed,
        limits,
        deadline,
    )?;
    revalidate_unchanged_since_sample(discovered, &upserts, limits, deadline)?;

    let revision_id = revision::publish_revision(
        connection,
        &revision::RevisionInputs {
            expected_current,
            manifest_hash: &diff.manifest_hash,
            parser_pack_version,
            upserts: &upserts,
            deletions: &deletions,
            skipped: &skipped_records,
            discovered_paths: &discovered_paths,
            re_resolve_unresolved,
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
