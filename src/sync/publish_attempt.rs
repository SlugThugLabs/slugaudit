// slugaudit-line-exception: approved-by=split-judge; reason=single publish attempt orchestrating Pass 1 diff, unlocked Pass 2 staging, and CAS commit
//! Single publish attempt: Pass 1 walk + single-open sniff/hash + diff,
//! uncontended Pass 2 chunked sampling/parsing into [`super::staging::StagingDb`]
//! on the persistent worker pool, `BeforeCommit` hook, and a short
//! `BEGIN IMMEDIATE` CAS commit transaction on `project.db`.

use super::discovery::{self, SkippedFile};
use super::hash_batch::{self, SniffedWalk};
use super::manifest::{ChangeStatus, FileChange};
use super::publish::{PublishError, PublishReport};
use super::publish_diff::{self, filter_skipped_records, non_deleted_paths};
use super::race_hook::{self, RacePoint};
use super::revision::{self, RevisionError, SkippedRecord};
use super::revision_edges::{EdgeChangeScope, StagedFileEdgeMeta, is_cargo_toml_path};
use super::revision_rust_meta::any_path_has_stored_rust_path_attr;
#[cfg(test)]
use super::sample;
use super::sample_batch::{self, ChunkOutcome, Pass2ChunkContext};
use super::staging::StagingDb;
use crate::model::{ResourceLimits, SourceIdentity};
use crate::progress::ProgressSink;
use crate::util::Deadline;
use rusqlite::{Connection, TransactionBehavior};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicU64;

/// Everything Pass 1 (walk + single-pass sniff/hash + diff) produces.
struct DiffState {
    discovered: Vec<discovery::DiscoveredFile>,
    discovered_paths: HashSet<String>,
    identities: Vec<SourceIdentity>,
    diff: publish_diff::Diff,
    skipped: Vec<SkippedFile>,
    skipped_records: BTreeMap<String, SkippedRecord>,
    unrepresentable: HashSet<Vec<u8>>,
}

/// Selects the discovered files Pass 2 must read: every hashed file when the
/// parser version changed, otherwise only the hashed files marked Added or
/// Modified by the diff.
pub(super) fn files_to_sample(
    discovered: &[discovery::DiscoveredFile],
    identities: &[SourceIdentity],
    changes: &[FileChange],
    parser_version_changed: bool,
) -> Vec<discovery::DiscoveredFile> {
    let hashed_paths: HashSet<&str> = identities
        .iter()
        .map(|id| id.relative_path.as_str())
        .collect();
    let changed_set = (!parser_version_changed).then(|| non_deleted_paths(changes));
    discovered
        .iter()
        .filter(|file| {
            let path = file.relative_path.as_str();
            hashed_paths.contains(path)
                && changed_set
                    .as_ref()
                    .is_none_or(|changed| changed.contains(path))
        })
        .cloned()
        .collect()
}

/// Validates that no sampled file changed between Pass 1 and Pass 2 (test helper).
#[cfg(test)]
pub(super) fn check_samples_match_pass1(
    samples: &[sample::Sample],
    identities: &[SourceIdentity],
) -> Result<(), PublishError> {
    let pass1_hashes: HashMap<&str, &str> = identities
        .iter()
        .map(|id| (id.relative_path.as_str(), id.content_hash.as_str()))
        .collect();
    for sample in samples {
        if let Some(&p1_hash) = pass1_hashes.get(sample.relative_path.as_str())
            && sample.identity.content_hash != p1_hash
        {
            return Err(PublishError::ChangedDuringSample {
                path: sample.relative_path.clone(),
            });
        }
    }
    Ok(())
}

/// Pass 1: Walk regular files, sniff binary + BLAKE3-hash in one file open per
/// file on the bounded worker pool, then diff against stored hashes.
fn hash_and_diff(
    connection: &Connection,
    root: &Path,
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<DiffState, PublishError> {
    let (walked, discovery_skips) = discovery::walk_with_deadline(root, deadline)?;
    race_hook::fire(root, RacePoint::AfterDiscovery);
    let unrepresentable = discovery::unrepresentable_paths(root, &discovery_skips);

    let SniffedWalk {
        discovered,
        identities,
        skips: hashing_skips,
    } = hash_batch::sniff_and_hash_walked(&walked, limits.max_file_bytes, deadline)?;
    drop(walked);

    let mut skipped = discovery_skips;
    skipped.extend(hashing_skips);

    let mut discovered_paths: HashSet<String> = discovered
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    let mut skipped_records: BTreeMap<String, SkippedRecord> = BTreeMap::new();
    publish_diff::collect_skipped(
        root,
        &skipped,
        None,
        &mut discovered_paths,
        &mut skipped_records,
    )?;
    let skipped_records_vec: Vec<SkippedRecord> = skipped_records.values().cloned().collect();

    let diff = publish_diff::diff_against_stored(
        connection,
        &discovered,
        &identities,
        &skipped_records_vec,
        deadline,
    )?;

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

fn unchanged_report(
    connection: &Connection,
    root: &Path,
    baseline: &CurrentRevision,
    unchanged: usize,
    skipped: Vec<SkippedFile>,
) -> Result<PublishReport, PublishError> {
    race_hook::fire(root, RacePoint::BeforeCommit);
    let current_now = current_revision(connection)?;
    let current_id_now = current_now.as_ref().map(|r| r.revision_id.as_str());
    if current_id_now != Some(baseline.revision_id.as_str()) {
        return Err(PublishError::Revision(RevisionError::StaleBaseline {
            expected: baseline.revision_id.clone(),
            found: revision::baseline_label(current_id_now),
        }));
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

struct StagedPass2 {
    staging: Option<StagingDb>,
    staged_metas: Vec<StagedFileEdgeMeta>,
    sampling_skips: Vec<SkippedFile>,
    cargo_toml_upserted: bool,
}

/// Runs Pass 2 across all 256-file chunks into an uncontended [`StagingDb`]
/// while `project.db` remains completely unlocked.
fn stage_pass2_outside_lock(
    root: &Path,
    files_to_sample: &[discovery::DiscoveredFile],
    ctx: &Pass2ChunkContext<'_>,
) -> Result<StagedPass2, PublishError> {
    if files_to_sample.is_empty() {
        return Ok(StagedPass2 {
            staging: None,
            staged_metas: Vec::new(),
            sampling_skips: Vec::new(),
            cargo_toml_upserted: false,
        });
    }
    let mut staging = StagingDb::open(root)?;
    let mut sampling_skips = Vec::new();
    let mut staged_metas = Vec::with_capacity(files_to_sample.len());
    let mut cargo_toml_upserted = false;
    let mut next_seq = 0_i64;

    sample_batch::run_pass2_chunks(files_to_sample, ctx, |outcomes| {
        let chunk_tx = staging.transaction().map_err(RevisionError::Database)?;
        for outcome in outcomes {
            match outcome {
                ChunkOutcome::Skipped(skip) => sampling_skips.push(skip),
                ChunkOutcome::Parsed(record) => {
                    revision::check_deadline(Some(ctx.deadline))?;
                    if is_cargo_toml_path(&record.relative_path) {
                        cargo_toml_upserted = true;
                    }
                    let seq = next_seq;
                    next_seq =
                        next_seq
                            .checked_add(1)
                            .ok_or(PublishError::SamplingIndexOutOfRange {
                                index: ctx.total_files,
                                total: ctx.total_files,
                            })?;
                    StagingDb::stage_file(&chunk_tx, seq, &record)?;
                    staged_metas.push(StagedFileEdgeMeta::from_record(seq, &record));
                }
            }
        }
        chunk_tx.commit().map_err(RevisionError::Database)?;
        Ok(())
    })?;

    SkippedFile::sort_by_path(&mut sampling_skips);
    Ok(StagedPass2 {
        staging: Some(staging),
        staged_metas,
        sampling_skips,
        cargo_toml_upserted,
    })
}

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
    let unrepresentable_changed =
        revision::stored_unrepresentable_paths(connection)? != state.unrepresentable;

    let parser_version_changed = baseline
        .as_ref()
        .is_none_or(|revision| revision.parser_pack_version != parser_pack_version);

    if state.diff.changes.is_empty()
        && !parser_version_changed
        && !unrepresentable_changed
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

    let to_sample = files_to_sample(
        &state.discovered,
        &state.identities,
        &state.diff.changes,
        parser_version_changed,
    );

    stage_and_commit_attempt(
        connection,
        StageCommitInputs {
            root,
            expected_current,
            parser_pack_version,
            state,
            files_to_sample: &to_sample,
            parser_version_changed,
            sink,
            limits,
            deadline,
        },
    )
}

struct StageCommitInputs<'a> {
    root: &'a Path,
    expected_current: Option<&'a str>,
    parser_pack_version: &'a str,
    state: DiffState,
    files_to_sample: &'a [discovery::DiscoveredFile],
    parser_version_changed: bool,
    sink: &'a dyn ProgressSink,
    limits: &'a ResourceLimits,
    deadline: &'a Deadline,
}

fn stage_and_commit_attempt(
    connection: &mut Connection,
    inputs: StageCommitInputs<'_>,
) -> Result<PublishReport, PublishError> {
    let StageCommitInputs {
        root,
        expected_current,
        parser_pack_version,
        state,
        files_to_sample,
        parser_version_changed,
        sink,
        limits,
        deadline,
    } = inputs;
    let DiffState {
        discovered: _,
        mut discovered_paths,
        identities,
        diff,
        mut skipped,
        mut skipped_records,
        unrepresentable,
    } = state;

    revision::check_deadline(Some(deadline))?;
    let deletions: Vec<String> = diff
        .changes
        .iter()
        .filter(|change| change.status == ChangeStatus::Deleted)
        .map(|change| change.relative_path.clone())
        .collect();
    let pass1_hashes: HashMap<&str, &str> = identities
        .iter()
        .map(|id| (id.relative_path.as_str(), id.content_hash.as_str()))
        .collect();

    let total_bytes = AtomicU64::new(0);
    let chunk_ctx = Pass2ChunkContext {
        total_files: files_to_sample.len(),
        pass1_hashes: &pass1_hashes,
        limits,
        total_bytes: &total_bytes,
        sink,
        deadline,
    };
    let mut staged = stage_pass2_outside_lock(root, files_to_sample, &chunk_ctx)?;

    publish_diff::collect_skipped(
        root,
        &staged.sampling_skips,
        Some(&identities),
        &mut discovered_paths,
        &mut skipped_records,
    )?;
    skipped.extend(std::mem::take(&mut staged.sampling_skips));
    let skipped_records_vec: Vec<SkippedRecord> = skipped_records.into_values().collect();
    let rewrite_all = expected_current.is_none() || parser_version_changed;
    let skipped_to_write = filter_skipped_records(skipped_records_vec, &diff.changes, rewrite_all);
    let edge_scope = EdgeChangeScope::from_changes(&diff.changes, staged.cargo_toml_upserted);

    race_hook::fire(root, RacePoint::BeforeCommit);
    let revision_id = commit_staged_attempt(
        connection,
        CommitInputs {
            expected_current,
            manifest_hash: &diff.manifest_hash,
            parser_pack_version,
            deletions: &deletions,
            skipped_to_write: &skipped_to_write,
            discovered_paths: &discovered_paths,
            unrepresentable: &unrepresentable,
            staged,
            edge_scope,
            deadline,
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

struct CommitInputs<'a> {
    expected_current: Option<&'a str>,
    manifest_hash: &'a str,
    parser_pack_version: &'a str,
    deletions: &'a [String],
    skipped_to_write: &'a [SkippedRecord],
    discovered_paths: &'a HashSet<String>,
    unrepresentable: &'a HashSet<Vec<u8>>,
    staged: StagedPass2,
    edge_scope: EdgeChangeScope,
    deadline: &'a Deadline,
}

fn commit_staged_attempt(
    connection: &mut Connection,
    inputs: CommitInputs<'_>,
) -> Result<String, RevisionError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    revision::assert_baseline(&tx, inputs.expected_current)?;
    let (revision_db_id, revision_id) =
        revision::insert_revision_row(&tx, inputs.manifest_hash, inputs.parser_pack_version)?;
    let removed_or_skipped = inputs.deletions.iter().map(String::as_str).chain(
        inputs
            .skipped_to_write
            .iter()
            .map(|s| s.relative_path.as_str()),
    );
    let deleted_or_skipped_had_path_attr =
        any_path_has_stored_rust_path_attr(&tx, removed_or_skipped)?;
    revision::apply_deletions(&tx, inputs.deletions, Some(inputs.deadline))?;

    let edge_inputs = match inputs.staged.staging.as_ref() {
        Some(staging) => revision::copy_staged_files_to_tx(
            &tx,
            staging,
            &inputs.staged.staged_metas,
            revision_db_id,
            inputs.edge_scope.any_rust_or_manifest_added_or_deleted(),
            Some(inputs.deadline),
        )?,
        None => Vec::new(),
    };
    drop(inputs.staged.staging);

    revision::upsert_skipped_files(
        &tx,
        inputs.skipped_to_write,
        revision_db_id,
        Some(inputs.deadline),
    )?;
    revision::replace_unrepresentable_paths_in_tx(&tx, inputs.unrepresentable)?;
    revision::finish_edges_and_flip(
        &tx,
        &edge_inputs,
        inputs.discovered_paths,
        inputs.edge_scope,
        deleted_or_skipped_had_path_attr,
        revision_db_id,
        Some(inputs.deadline),
    )?;
    tx.commit()?;
    Ok(revision_id)
}

#[cfg(test)]
#[path = "publish_sample_tests.rs"]
mod sample_tests;
