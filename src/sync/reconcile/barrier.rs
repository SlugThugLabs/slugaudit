//! The barrier synchronization loop: snapshot dirty/deleted sets, run
//! the reconcile pipeline, and if new events arrived during
//! reconciliation, loop until the watcher sequence stabilizes. Bounded by
//! [`MAX_BARRIER_LOOPS`] so a pathological event producer fails closed by
//! marking the watcher `Desynced` instead of draining forever.

use super::{MAX_BARRIER_LOOPS, ReconcileError};
#[cfg(test)]
use super::{ReconcileOptions, ReconcileReport};
#[cfg(test)]
use crate::model::process_limits;
use crate::util::Deadline;
use crate::watch::WatchState;
#[cfg(test)]
use rusqlite::Connection;
use std::collections::HashSet;
#[cfg(test)]
use std::path::Path;

/// Implements the barrier synchronization loop.
///
/// Snapshots the dirty/deleted sets (without acknowledging), reconciles
/// them, and then checks if new events arrived during reconciliation. If
/// so, loops and reconciles the new events. Continues until the watcher
/// sequence stabilizes, then acknowledges through the final sequence.
///
/// Bounded by [`MAX_BARRIER_LOOPS`]: exceeding the cap signals an
/// external producer racing reconciliation and surfaces as a
/// [`ReconcileError::BarrierCapExceeded`] after marking the watcher
/// `Desynced` so subsequent calls do a full verification rather than
/// spinning. The error is not logged here — the caller's terminal failure
/// handler logs it once, so one failure never produces two warn lines.
///
/// If `reconcile_fn` fails, the error propagates and the dirty sets remain
/// unacknowledged — the caller is responsible for marking the watcher
/// untrusted so the next call re-verifies.
#[cfg(test)]
pub(crate) fn sync_with_barrier(
    state: &WatchState,
    reconcile_fn: impl FnMut(HashSet<String>, HashSet<String>) -> Result<(), ReconcileError>,
) -> Result<(), ReconcileError> {
    sync_with_barrier_with_deadline(
        state,
        &Deadline::after(process_limits().max_sync_wall_clock),
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
    state: &WatchState,
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
                path: String::new(),
                elapsed_ms: elapsed.as_millis(),
            });
        }

        reconcile_fn(dirty, deleted)?;
        iterations += 1;

        if iterations >= MAX_BARRIER_LOOPS {
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

/// Reconciles dirty and deleted paths against the database for tests that
/// exercise the pipeline directly, without a watcher or a barrier loop.
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
#[cfg(test)]
pub(crate) fn reconcile_dirty_paths(
    connection: &mut Connection,
    root: &Path,
    dirty: HashSet<String>,
    deleted: HashSet<String>,
    expected_revision: Option<&str>,
) -> Result<ReconcileReport, ReconcileError> {
    let options = ReconcileOptions::for_sync(root, None);
    super::pipeline::reconcile_dirty_paths_with_deadline(
        connection,
        root,
        dirty,
        deleted,
        expected_revision,
        &options,
        // No walk in hand: fixtures index every file they write, so the
        // indexed rows are the correct universe for these tests.
        &HashSet::new(),
    )
}
