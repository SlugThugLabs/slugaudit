//! The per-path reconciliation pipeline: for each dirty path, decide
//! delete / ignore / unchanged / re-index, then atomically commit the
//! resulting upserts and deletions under compare-and-swap. The barrier
//! loop that drives this pipeline lives in [`super::barrier`]; the SQL
//! it reads lives in [`super::queries`].

use super::queries::{query_current_parser_pack_version, query_existing_hashes};
use super::{ReconcileError, ReconcileOptions, ReconcileReport};
use crate::sync::discovery::{DiscoveredFile, sniff_kind};
use crate::sync::{hash, manifest, revision, sample};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

/// Reconciles dirty and deleted paths against the database.
///
/// For each dirty path, reads the file, computes its BLAKE3 hash, and
/// queries the database for the previously stored hash. If the hash
/// matches, the file is skipped (unchanged since last index). If the hash
/// differs or the file is new, it is sampled, parsed, analyzed, and added
/// to the upsert set. Deleted paths are added to the deletion set.
///
/// After processing all paths, the function calls `publish_revision` to
/// atomically commit the upserts and deletions under compare-and-swap.
///
/// # Errors
///
/// Returns an error if any file read, hash, sample, analysis, or database
/// operation fails.
pub fn reconcile_dirty_paths(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
) -> Result<ReconcileReport, ReconcileError> {
    let options = ReconcileOptions::for_sync(None);
    reconcile_dirty_paths_with_deadline(
        connection,
        root,
        dirty,
        deleted,
        expected_revision,
        &options,
    )
}

/// [`reconcile_dirty_paths`] under explicit [`ReconcileOptions`], checked
/// once per dirty path so a pathological set (a huge number of events, a
/// hung filesystem) fails closed with
/// [`ReconcileError::TimeBudgetExceeded`] instead of stalling the tool
/// call forever. The deadline is created by the caller so the whole
/// barrier-sync operation shares one budget across its iterations.
pub(crate) fn reconcile_dirty_paths_with_deadline(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
    options: &ReconcileOptions,
) -> Result<ReconcileReport, ReconcileError> {
    let ReconcileOptions {
        limits,
        deadline,
        rules,
    } = options;
    let mut upserts = Vec::new();
    let mut deletions = Vec::new();
    let mut reconciled = 0usize;
    let mut unchanged = 0usize;
    let mut ignored = 0usize;
    let mut skipped = 0usize;

    // Query existing hashes for dirty paths so we can skip files whose
    // content hasn't changed since they were last indexed.
    let existing_hashes = query_existing_hashes(connection, &dirty)?;

    for path in dirty {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(ReconcileError::TimeBudgetExceeded {
                path: format!(" while processing {path}"),
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let absolute_path = root.join(&path);

        // If the file no longer exists on disk, treat it as a deletion.
        // This handles the race where a file was marked dirty and then
        // deleted before we could reconcile it.
        if !absolute_path.exists() {
            deletions.push(path);
            continue;
        }
        // Skip paths the project's ignore rules exclude. A gitignored
        // build artifact must not be indexed incrementally when a fresh
        // publish would skip it — this was the watcher/full-publish
        // inconsistency. Deletions are still processed (above) so a file
        // that became ignored — or was indexed before the rules existed —
        // converges out of the database.
        if let Some(rules) = rules
            && rules.should_ignore(&path)
        {
            ignored += 1;
            continue;
        }

        // Stat BEFORE hashing: the stored stat fingerprint must never be
        // newer than the content it describes. A file modified between its
        // hash-read and a post-read stat would store the *new* mtime against
        // the *old* content hash, and the next sweep would skip it as
        // unchanged — serving stale evidence. A pre-read stat can only err
        // stale (one extra nomination next sweep), never fresh.
        let pre_read_stat = std::fs::metadata(&absolute_path).ok().map(|metadata| {
            (
                crate::util::mtime_unix_seconds(&metadata),
                i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            )
        });

        let identity = hash::hash_file(&path, &absolute_path)?;

        if let Some(existing_hash) = existing_hashes.get(&path)
            && existing_hash == &identity.content_hash
        {
            unchanged += 1;
            // Content is unchanged; only the stat fingerprint drifted
            // (editor no-op save, touch, git checkout). Refresh it so the
            // sweep stops re-nominating this path on every call. Safe
            // outside the publish transaction: the fingerprint describes
            // content that is already committed and unchanged.
            if let Some((mtime, byte_len)) = pre_read_stat {
                connection.execute(
                    "UPDATE files SET modified_unix_seconds = ?1, byte_len = ?2 WHERE path = ?3",
                    rusqlite::params![mtime, byte_len, path],
                )?;
            }
            continue;
        }

        // Hash differs or file is new — re-sniff binary-ness exactly the
        // way the initial import's discovery does, then sample, parse,
        // analyze, and add to the upsert set. Hardcoding `Indexed` here
        // would re-index a modified binary file as lossy UTF-8 text.
        let kind = sniff_kind(&absolute_path)?;
        let discovered = DiscoveredFile {
            relative_path: path.clone(),
            absolute_path: absolute_path.clone(),
            kind,
        };
        let sample = match sample::sample_file(&discovered, limits) {
            Ok(sample) => sample,
            // Per-file import ceiling: the file cannot be indexed at all.
            // Skip it the same way a full publish now skips oversized
            // files (sample_batch.rs) and report the count — never fail
            // the whole reconcile over one file.
            Err(sample::SampleError::TooLarge { .. }) => {
                skipped += 1;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let record = sample::to_file_record(sample, limits);
        upserts.push(record);
        reconciled += 1;
    }

    for path in deleted {
        deletions.push(path);
    }

    // Nothing to commit — avoid churning out a no-op revision.
    if upserts.is_empty() && deletions.is_empty() {
        return Ok(ReconcileReport {
            reconciled,
            unchanged,
            deleted: 0,
            ignored,
            skipped,
        });
    }

    let manifest_hash = manifest::compute_manifest_hash(connection, &upserts, &deletions)?;
    let parser_pack_version = query_current_parser_pack_version(connection)?;

    let _revision_id = revision::publish_revision(
        connection,
        expected_revision,
        &manifest_hash,
        &parser_pack_version,
        &upserts,
        &deletions,
    )?;

    Ok(ReconcileReport {
        reconciled,
        unchanged,
        deleted: deletions.len(),
        ignored,
        skipped,
    })
}
