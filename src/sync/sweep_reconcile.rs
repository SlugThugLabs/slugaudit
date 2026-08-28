//! The sweep orchestrator: run the stat sweep, then reconcile what it
//! found under one shared wall-clock budget. Split from `manager.rs`
//! (the same pattern as `manager_meta.rs`) so the sync orchestrator stays
//! under the small-file rule cap. Owns the failure contract too: a failed
//! sweep leaves freshness ambiguous, so the watcher is marked `Desynced`
//! and the progress lifecycle is closed before the error propagates.

use super::manager_meta::current_revision_id;
use super::reconcile::{ReconcileOptions, reconcile_dirty_paths_with_deadline};
use super::sweep::{SweepError, sweep};
use crate::ignore_rules::IgnoreRules;
use crate::model::process_limits;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::util::Deadline;
use crate::watch::{WatchState, WatcherHealth};
use rmcp::ErrorData;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;

/// Runs the stat sweep and reconciles what it found: new files, stat
/// mismatches, and deletions converge the database to disk state with a
/// compare-and-swap publish, exactly like a watcher-driven reconcile.
/// Candidate re-hashes skip unchanged content, so a sweep over an unchanged
/// project costs one stat per file and publishes nothing.
///
/// On any failure the watcher is marked [`WatcherHealth::Desynced`] and the
/// progress lifecycle is closed — the next `ensure_current` does a full
/// verification instead of silently serving stale evidence.
pub(super) fn sweep_and_reconcile(
    root: &Path,
    rules: Option<Arc<IgnoreRules>>,
    connection: &mut Connection,
    state: &WatchState,
    sink: &dyn ProgressSink,
) -> Result<(), ErrorData> {
    match sweep_then_reconcile(root, rules, connection) {
        Ok((candidates, deleted, unchanged)) => {
            if candidates > 0 || deleted > 0 {
                tracing::debug!(
                    root = %root.display(),
                    candidates,
                    deleted,
                    unchanged,
                    "stat sweep reconciled changes the watcher did not report",
                );
            }
            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                root = %root.display(),
                error = %error,
                "stat sweep failed; marking watcher Desynced so the next call re-verifies",
            );
            state.set_health(WatcherHealth::Desynced);
            sink.emit(ProgressEvent::Completed {
                phase: "ensuring_current",
            });
            Err(error)
        }
    }
}

/// The sweep plus its reconcile under one deadline. `unchanged` counts the
/// files the sweep proved unchanged with a single stat — the observability
/// signal that the backstop is cheap, not just correct.
fn sweep_then_reconcile(
    root: &Path,
    rules: Option<Arc<IgnoreRules>>,
    connection: &mut Connection,
) -> Result<(usize, usize, usize), ErrorData> {
    let limits = *process_limits();
    // One deadline for the sweep walk and its reconcile together, so a
    // pathological tree fails closed instead of stalling the tool call.
    let deadline = Deadline::after(limits.max_sync_wall_clock);
    let report = sweep(connection, root, &deadline)
        .map_err(|error| ErrorData::internal_error(sweep_error_text(error), None))?;
    let unchanged = report.unchanged;
    if report.candidates.is_empty() && report.deleted.is_empty() {
        return Ok((0, 0, unchanged));
    }
    let expected = current_revision_id(connection).map_err(|error| {
        ErrorData::internal_error(
            format!("reading the current revision before the sweep reconcile: {error}"),
            None,
        )
    })?;
    let options = ReconcileOptions {
        limits,
        deadline,
        rules,
    };
    let candidates = report.candidates.len();
    let deleted = report.deleted.len();
    reconcile_dirty_paths_with_deadline(
        connection,
        root,
        report.candidates,
        report.deleted,
        expected.as_deref(),
        &options,
    )
    .map_err(|error| {
        ErrorData::internal_error(format!("reconciling sweep results: {error}"), None)
    })?;
    Ok((candidates, deleted, unchanged))
}

/// Sweeps share the reconcile pipeline's error vocabulary; the budget
/// variant names itself so an operator reading the failure can tell a hung
/// filesystem from a database fault.
fn sweep_error_text(error: SweepError) -> String {
    match error {
        SweepError::TimeBudgetExceeded { elapsed_ms } => {
            format!("stat sweep exceeded its wall-clock budget ({elapsed_ms} ms)")
        }
        SweepError::Database(database_error) => format!("stat sweep: {database_error}"),
    }
}
