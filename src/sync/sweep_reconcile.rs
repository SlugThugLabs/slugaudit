//! The sweep-driven reconcile: apply the stat sweep's findings under the
//! caller's compare-and-swap revision read. Split from `manager.rs` (the
//! same pattern as `manager_meta.rs`) so the sync orchestrator stays under
//! the small-file rule cap. The walk and the apply are separate on purpose:
//! the Healthy path walks the tree exactly once and reuses that report for
//! both the event-driven reconcile's `on_disk` and the apply below, so the
//! edit-then-query hot path never pays two whole-tree walks. Owns the
//! failure contract too: a failed sweep leaves freshness ambiguous, so the
//! watcher is marked `Desynced` and the progress lifecycle is closed before
//! the error propagates.

use super::discovery::unrepresentable_relative_bytes;
use super::manager_meta::current_revision_id;
use super::reconcile::{ReconcileOptions, reconcile_dirty_paths_with_deadline};
use super::sweep::{SweepError, SweepReport, sweep};
use crate::error_codes;
use crate::ignore_rules::is_excluded_path;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::util::{Deadline, redacted_path};
use crate::watch::errors::{ignore_error_kind, ignore_error_path};
use crate::watch::{WatchState, WatcherHealth};
use rmcp::ErrorData;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

/// Walks `root` once under `deadline`, mapping the sweep's typed error into
/// the sync error vocabulary. Separated from the apply so a caller that also
/// needs the sweep's `on_disk` for an event-driven reconcile can reuse one
/// report instead of walking twice.
///
/// # Errors
///
/// Returns an error if the stored stat state can't be read or the walk
/// exhausts its budget — failing closed, because a partial sweep that
/// silently skipped the rest of the tree would serve the staleness the
/// sweep exists to prevent.
pub(super) fn sweep_tree(
    root: &Path,
    connection: &Connection,
    deadline: &Deadline,
) -> Result<SweepReport, ErrorData> {
    sweep(connection, root, deadline).map_err(|error| {
        let data = error_codes::budget_payload(error.budget_elapsed_ms());
        ErrorData::internal_error(sweep_error_text(error), data)
    })
}

/// Logs a sweep walker or ignore-file failure with the static category and
/// the real path and the error's `Display` as the cause. This is diagnostic context at debug
/// level: the single warn for the failure is [`mark_sweep_failed`]'s
/// operation line, so one failure never produces two warn lines. Returns the
/// category for the typed sweep failure.
pub(super) fn log_walk_failure(error: &ignore::Error) -> &'static str {
    let kind = ignore_error_kind(error);
    let path = ignore_error_path(error).map(|path| path.display().to_string());
    tracing::debug!(
        operation = "stat_sweep_walk",
        error_kind = kind,
        path = ?path,
        error = %error,
        "sweep could not read a directory entry or apply an ignore file; failing the \
         sweep so no row is deleted under an unverified subtree",
    );
    kind
}

/// The project-relative path a swept entry should be compared under, or
/// `None` when the entry is skipped: excluded, outside the root, or
/// non-UTF-8. A non-UTF-8 path is recorded in `unrepresentable` first —
/// discovery rejects such paths, so there is nothing to compare and nothing
/// to delete, but the persisted set `report` counts must stay current
/// between full publishes.
pub(super) fn walked_relative_path(
    root: &Path,
    absolute_path: &Path,
    unrepresentable: &mut HashSet<Vec<u8>>,
) -> Option<String> {
    let relative = match absolute_path.strip_prefix(root) {
        Ok(relative) => relative,
        Err(error) => {
            // The walker yields paths under `root`, so this cannot happen;
            // if it did, the entry is omitted and its stored row (if any)
            // would be classed deleted, so say so with the cause.
            tracing::warn!(
                path = %absolute_path.display(),
                error = %error,
                "sweep walked a path outside the project root; it is left out of the comparison",
            );
            return None;
        }
    };
    if is_excluded_path(&relative.to_string_lossy()) {
        return None;
    }
    let Some(relative) = relative.to_str().map(str::to_owned) else {
        unrepresentable.extend(unrepresentable_relative_bytes(root, absolute_path));
        return None;
    };
    Some(relative)
}

/// Which step of the sweep-driven reconcile failed. The walk and the apply
/// are distinct operations with distinct failure meanings — the walk could
/// not verify the tree, the apply verified it but could not publish the
/// result — so the failure log names the one that failed.
#[derive(Clone, Copy)]
enum SweepPhase {
    Walk,
    Apply,
}

impl SweepPhase {
    /// The static operation name carried in the failure log.
    const fn operation(self) -> &'static str {
        match self {
            SweepPhase::Walk => "stat_sweep_walk",
            SweepPhase::Apply => "sweep_reconcile_apply",
        }
    }

    /// The static failure message. Distinct per phase so a log reader can
    /// tell a failed walk from a failed apply without parsing the error.
    const fn message(self) -> &'static str {
        match self {
            SweepPhase::Walk => {
                "stat sweep walk failed; marking watcher Desynced so the next call re-verifies"
            }
            SweepPhase::Apply => {
                "sweep reconcile apply failed; marking watcher Desynced so the next call \
                 re-verifies"
            }
        }
    }
}

/// The failure contract for a failed walk or apply: freshness is ambiguous,
/// so the watcher is marked [`WatcherHealth::Desynced`] and the progress
/// lifecycle is closed before the error propagates — the next
/// `ensure_current` full-verifies instead of silently serving unverified
/// evidence.
pub(super) fn sweep_failed(
    root: &Path,
    state: &WatchState,
    sink: &dyn ProgressSink,
    error: ErrorData,
) -> ErrorData {
    mark_sweep_failed(root, state, sink, SweepPhase::Walk, error)
}

/// [`sweep_failed`] for the apply step, whose failure is not a failed walk.
fn sweep_apply_failed(
    root: &Path,
    state: &WatchState,
    sink: &dyn ProgressSink,
    error: ErrorData,
) -> ErrorData {
    mark_sweep_failed(root, state, sink, SweepPhase::Apply, error)
}

/// The shared failure contract. Logs once with the failing phase's static
/// operation and message plus the error's closed-set machine code, then
/// closes the progress lifecycle. The error's `Display` is never logged: a
/// per-path budget note can embed a project-relative path. The typed error
/// itself is returned unchanged to the caller.
fn mark_sweep_failed(
    root: &Path,
    state: &WatchState,
    sink: &dyn ProgressSink,
    phase: SweepPhase,
    error: ErrorData,
) -> ErrorData {
    let error_code = error
        .data
        .as_ref()
        .and_then(|data| data["code"].as_str())
        .unwrap_or("none");
    tracing::warn!(
        root_id = %redacted_path(root),
        operation = phase.operation(),
        error_code,
        "{}",
        phase.message(),
    );
    state.set_health(WatcherHealth::Desynced);
    sink.emit(ProgressEvent::Completed {
        phase: "ensuring_current",
    });
    error
}

/// Applies a sweep report's findings — its candidate re-hashes and its
/// deletions — with a compare-and-swap publish, exactly like a
/// watcher-driven reconcile. The expected revision is read here, at the
/// point this step publishes, so it is never shared with the event-driven
/// reconcile's baseline; a race between the two surfaces as the existing
/// `StaleBaseline` retry. The walk belongs to the caller: the report carries
/// `on_disk`, so edge resolution treats discovered-but-skipped files as
/// existing. Returns `(candidates, deleted, unchanged)` for the caller's log.
///
/// # Errors
///
/// Returns an error if reading the current revision or reconciling the
/// report's candidate/deleted paths fails.
pub(super) fn apply_sweep_report(
    root: &Path,
    connection: &mut Connection,
    report: SweepReport,
    options: &ReconcileOptions,
) -> Result<(usize, usize, usize), ErrorData> {
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
    let candidates = report.candidates.len();
    let deleted = report.deleted.len();
    reconcile_dirty_paths_with_deadline(
        connection,
        root,
        report.candidates,
        report.deleted,
        expected.as_deref(),
        options,
        &report.on_disk,
    )
    .map_err(|error| {
        let data = error_codes::budget_payload(error.budget_elapsed_ms());
        ErrorData::internal_error(format!("reconciling sweep results: {error}"), data)
    })?;
    Ok((candidates, deleted, unchanged))
}

/// Applies a precomputed sweep report and owns its failure contract:
/// [`apply_sweep_report`], then the findings log, and — on failure — the
/// `Desynced` marking and progress-lifecycle close that keep a failed sweep
/// from serving unverified evidence. The walk itself stays with the caller
/// ([`sweep_tree`]), so the Healthy path walks once and reuses that report
/// for the event-driven reconcile too.
///
/// # Errors
///
/// Returns the failure-contract error if the apply fails.
pub(super) fn sweep_and_reconcile(
    root: &Path,
    report: SweepReport,
    options: &ReconcileOptions,
    connection: &mut Connection,
    state: &WatchState,
    sink: &dyn ProgressSink,
) -> Result<(), ErrorData> {
    match apply_sweep_report(root, connection, report, options) {
        Ok((candidates, deleted, unchanged)) => {
            log_sweep_findings(root, candidates, deleted, unchanged);
            Ok(())
        }
        Err(error) => Err(sweep_apply_failed(root, state, sink, error)),
    }
}

/// Logs the sweep's findings. `unchanged` counts the files the sweep proved
/// unchanged with a single stat — the observability signal that the backstop
/// is cheap, not just correct. Silent when the sweep found nothing, so an
/// unchanged project pays no log line.
pub(super) fn log_sweep_findings(root: &Path, candidates: usize, deleted: usize, unchanged: usize) {
    if candidates == 0 && deleted == 0 {
        return;
    }
    tracing::debug!(
        root_id = %redacted_path(root),
        candidates,
        deleted,
        unchanged,
        "stat sweep reconciled changes the watcher did not report",
    );
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
        SweepError::IncompleteWalk { kind } => {
            format!("stat sweep could not verify the tree ({kind}); no rows were deleted")
        }
    }
}
