//! Reconciliation logic for SlugAudit's watch-based sync.
//!
//! When the filesystem watcher reports dirty or deleted paths, this module
//! reconciles them against the database: hashing dirty files, comparing
//! with stored hashes, and re-indexing only files that actually changed.
//! The barrier synchronization loop ensures no events are lost even if
//! new events arrive during reconciliation.
// slugaudit-line-exception: approved-by=agent; reason=the per-path reconcile loop, its barrier sync, and the report types are one atomic pipeline; manifest-hash computation lives in manifest.rs and the barrier-cap test belongs next to the loop it covers

use super::discovery::{DiscoveredFile, DiscoveryError};
use super::hash;
use super::revision;
use super::sample;
use crate::model::ResourceLimits;
use crate::util::Deadline;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Hash(#[from] super::hash::HashError),
    #[error(transparent)]
    Sample(#[from] super::sample::SampleError),
    #[error(transparent)]
    Revision(#[from] super::revision::RevisionError),
    /// Barrier sync hit its iteration cap. The watcher is being marked
    /// `Desynced` so the next sync call falls back to a full verification
    /// rather than continuing to drain an endless stream of racing events.
    /// Reaching this is a clear signal of a pathological producer
    /// (editor saving faster than reconcile completes, fsmonitor firing
    /// repeatedly, etc.) — preferable to looping forever and exhausting
    /// memory.
    #[error(
        "barrier sync hit the {iterations}-iteration cap, watching is marked Desynced \
         and the next call will do a full verification"
    )]
    BarrierCapExceeded { iterations: u32 },
    #[error("reconcile exceeded its wall-clock time budget after {elapsed_ms} ms")]
    TimeBudgetExceeded { elapsed_ms: u128 },
}

/// Report of a reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    /// Files that were re-indexed because their hash differed from the
    /// stored hash or because they were new to the database.
    pub reconciled: usize,
    /// Files whose hash matched the stored hash and were skipped.
    pub unchanged: usize,
    /// Files that were removed from the database.
    pub deleted: usize,
    /// Dirty paths the project's ignore rules excluded — not indexed, the
    /// same way a fresh publish would skip them.
    pub ignored: usize,
    /// Dirty paths skipped because they exceed `limits.max_file_bytes` —
    /// recorded so the report stays honest about what was not indexed and
    /// why, mirroring the full-publish path's per-file skips (the file
    /// cannot be indexed at all, and it must not fail the whole reconcile).
    pub skipped: usize,
}

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

/// The reconciliation context every dirty path is checked against,
/// distinct from the working set (connection, root, dirty, deleted,
/// expected revision): the resource budget, the shared deadline, and the
/// project's ignore rules. Grouped so callers can't mix up budget and
/// rule concerns and to keep the function signature readable.
pub(crate) struct ReconcileOptions {
    pub limits: ResourceLimits,
    pub deadline: Deadline,
    pub rules: Option<Arc<crate::ignore_rules::IgnoreRules>>,
}

impl ReconcileOptions {
    /// Options for a production sync pass: the standard resource budget
    /// and deadline, plus the project's current ignore rules.
    pub fn for_sync(rules: Option<Arc<crate::ignore_rules::IgnoreRules>>) -> Self {
        let limits = ResourceLimits::default();
        let deadline = Deadline::after(limits.max_sync_wall_clock);
        Self {
            limits,
            deadline,
            rules,
        }
    }
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

        let identity = hash::hash_file(&path, &absolute_path)?;

        if let Some(existing_hash) = existing_hashes.get(&path)
            && existing_hash == &identity.content_hash
        {
            unchanged += 1;
            continue;
        }

        // Hash differs or file is new — re-sniff binary-ness exactly the
        // way the initial import's discovery does, then sample, parse,
        // analyze, and add to the upsert set. Hardcoding `Indexed` here
        // would re-index a modified binary file as lossy UTF-8 text.
        let kind = super::discovery::sniff_kind(&absolute_path)?;
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

    let manifest_hash = super::manifest::compute_manifest_hash(connection, &upserts, &deletions)?;
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

/// Maximum number of barrier-sync iterations before giving up. A
/// pathological editor that emits events faster than reconciliation
/// completes would otherwise loop forever, exhausting memory and never
/// returning to the caller. With this cap, the watcher is marked
/// `Desynced` and the next sync call falls back to a full verification.
pub const MAX_BARRIER_LOOPS: u32 = 16;

/// Implements the barrier synchronization loop.
///
/// Snapshots the dirty/deleted sets (without acknowledging), reconciles
/// them, and then checks if new events arrived during reconciliation. If
/// so, loops and reconciles the new events. Continues until the watcher
/// sequence stabilizes, then acknowledges through the final sequence.
///
/// Bounded by [`MAX_BARRIER_LOOPS`]: exceeding the cap signals an
/// external producer racing reconciliation, which is logged and surfaced
/// as a [`ReconcileError::BarrierCapExceeded`] after marking the watcher
/// `Desynced` so subsequent calls do a full verification rather than
/// spinning.
///
/// If `reconcile_fn` fails, the error propagates and the dirty sets remain
/// unacknowledged — the caller is responsible for marking the watcher
/// untrusted so the next call re-verifies.
pub fn sync_with_barrier(
    state: &crate::watch::WatchState,
    reconcile_fn: impl FnMut(HashSet<String>, HashSet<String>) -> Result<(), ReconcileError>,
) -> Result<(), ReconcileError> {
    sync_with_barrier_with_deadline(
        state,
        &Deadline::after(ResourceLimits::default().max_sync_wall_clock),
        reconcile_fn,
    )
}

/// [`sync_with_barrier`] under an explicit [`Deadline`], checked once per
/// iteration so a pathological producer (an editor saving faster than
/// reconcile completes) fails closed with
/// [`ReconcileError::TimeBudgetExceeded`] instead of draining events
/// indefinitely. The deadline is created by the caller so the whole
/// barrier-sync operation — every iteration, and every per-path reconcile
/// inside it — shares one budget.
pub(crate) fn sync_with_barrier_with_deadline(
    state: &crate::watch::WatchState,
    deadline: &Deadline,
    reconcile_fn: impl FnMut(HashSet<String>, HashSet<String>) -> Result<(), ReconcileError>,
) -> Result<(), ReconcileError> {
    let mut reconcile_fn = reconcile_fn;
    let mut iterations: u32 = 0;
    loop {
        let (seq, dirty, deleted) = state.snapshot_dirty();
        tracing::trace!(
            iteration = iterations,
            dirty_count = dirty.len(),
            deleted_count = deleted.len(),
            "barrier sync iteration"
        );

        if dirty.is_empty() && deleted.is_empty() {
            // Nothing left to reconcile is a success even if the budget is
            // spent — there is no work left to stall on.
            return Ok(());
        }
        if let Some(elapsed) = deadline.exceeded() {
            return Err(ReconcileError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }

        reconcile_fn(dirty, deleted)?;
        iterations += 1;

        if iterations >= MAX_BARRIER_LOOPS {
            tracing::warn!(
                iterations,
                "barrier sync hit its iteration cap; marking watcher Desynced",
            );
            state.set_health(crate::watch::WatcherHealth::Desynced);
            return Err(ReconcileError::BarrierCapExceeded {
                iterations: MAX_BARRIER_LOOPS,
            });
        }

        // Check if more events arrived during reconciliation.
        if state.current_sequence() == seq {
            // No new events — acknowledge through this sequence.
            state.acknowledge_through(seq);
            return Ok(());
        }
        // Otherwise, loop and reconcile the new events. The next
        // snapshot_dirty call will pick them up.
    }
}

/// Queries the stored content hashes for the given paths.
fn query_existing_hashes(
    connection: &Connection,
    paths: &HashSet<String>,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; paths.len()].join(", ");
    let sql = format!("SELECT path, content_hash FROM files WHERE path IN ({placeholders})");
    let params: Vec<&dyn rusqlite::ToSql> =
        paths.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
    let mut stmt = connection.prepare(&sql)?;
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut result = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        result.insert(path, hash);
    }
    Ok(result)
}

/// Reads the parser pack version of the current revision, falling back to
/// `"1.0"` when no revision has been published yet.
fn query_current_parser_pack_version(connection: &Connection) -> Result<String, rusqlite::Error> {
    let version: Option<String> = connection
        .query_row(
            "SELECT parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(version.unwrap_or_else(|| "1.0".to_owned()))
}

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reconcile_binary_tests.rs"]
mod binary_tests;

#[cfg(test)]
#[path = "reconcile_ignore_tests.rs"]
mod ignore_tests;
