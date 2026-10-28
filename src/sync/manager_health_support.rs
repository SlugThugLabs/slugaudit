//! Free functions behind the watcher-health state machine.
//!
//! [`super::manager`]'s `health` child owns the health transitions and the
//! [`super::SourceSyncManager`] methods; this module holds the pieces that do
//! not touch the manager's private state — the event barrier loop, the
//! post-sync revision read, and the typed sweep-error conversion. Keeping
//! them here leaves the state machine itself readable in one file.

use super::discovery::DiscoveryError;
use super::hash::HashError;
use super::manager::SyncError;
use super::manager_meta::current_revision_id;
use super::reconcile::{ReconcileError, ReconcileOptions};
use super::revision::RevisionError;
use super::sample::SampleError;
use super::sweep::SweepError;
use crate::watch::WatchState;
use rmcp::ErrorData;
use std::collections::HashSet;
use std::path::Path;

/// Reads the revision id the sync operation just converged on. Every field
/// is present or the call errors with a named reason — a successful sync
/// that cannot name its revision is a bug an operator must be able to see.
pub(super) fn revision_after_sync(
    connection: &rusqlite::Connection,
    database_path: &Path,
) -> Result<String, ErrorData> {
    current_revision_id(connection)
        .map_err(|error| {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                error = %error,
                "failed to read the current revision",
            );
            ErrorData::internal_error(format!("reading the current revision: {error}"), None)
        })?
        .ok_or_else(|| {
            tracing::warn!(
                database_id = %crate::util::redacted_path(database_path),
                "no current revision found after sync",
            );
            ErrorData::internal_error(
                "no current revision found after sync — this is unexpected; \
                 try disabling and re-enabling the project",
                None,
            )
        })
}

/// Reconciles watcher events under a caller-supplied deadline and on-disk
/// path set. The `Healthy` path walks the tree once for the sweep and passes
/// that report's `on_disk` here, so this never walks. The expected revision
/// is read inside the barrier loop, immediately before each iteration
/// publishes, so the CAS baseline is never shared with another publishing
/// step (the sweep apply reads its own immediately before it publishes).
///
/// # Errors
///
/// Returns an error if the barrier loop or any per-path reconcile fails.
pub(super) fn reconcile_events(
    root: &Path,
    state: &WatchState,
    connection: &mut rusqlite::Connection,
    options: &ReconcileOptions,
    on_disk: &HashSet<String>,
) -> Result<(), SyncError> {
    let started = std::time::Instant::now();
    // Accumulated across every barrier iteration so the log line below
    // reports the total skip count, not just the last pass's.
    let mut skipped_total = 0usize;

    super::reconcile::sync_with_barrier_with_deadline(
        state,
        &options.deadline,
        |dirty, deleted| {
            // Re-read the baseline for every iteration: an earlier
            // iteration may have published a revision, and a path that
            // changed again during that publish must be reconciled against
            // the revision that publication produced. Hoisting this read
            // out of the loop makes the second publishing iteration fail
            // with `RevisionError::StaleBaseline` instead of converging.
            let expected_current = current_revision_id(connection)?;
            let report = super::reconcile::reconcile_dirty_paths_with_deadline(
                connection,
                root,
                dirty,
                deleted,
                expected_current.as_deref(),
                options,
                on_disk,
            )?;
            skipped_total += report.skipped;
            Ok(())
        },
    )?;

    tracing::debug!(
        root_id = %crate::util::redacted_path(root),
        elapsed_ms = started.elapsed().as_millis(),
        skipped = skipped_total,
        "incremental reconcile phase complete",
    );
    Ok(())
}

/// The one sweep failure that must stay typed on its way into
/// [`SyncError`]: a spent budget carries `sync_budget_exhausted` and the
/// knob that raises it, and that code cannot be recovered from a flattened
/// string. Every other sweep failure converts losslessly to text, which is
/// all its response needs.
pub(super) fn sweep_error_to_sync(error: SweepError) -> SyncError {
    match error {
        SweepError::TimeBudgetExceeded { elapsed_ms } => {
            SyncError::SweepBudgetExceeded { elapsed_ms }
        }
        other => SyncError::Sweep(other.to_string()),
    }
}

/// Logs a sync failure with its `Display` form as the cause.
///
/// The record carries the real `root`, a static `error_kind`, the `path` the
/// variant names (when it names one), and `error`, the full cause chain
/// text (OS error, SQLite message), so a failure is diagnosable from the
/// log alone.
pub(super) fn log_sync_failure(
    operation: &'static str,
    root: &Path,
    message: &'static str,
    error: &SyncError,
) {
    let (error_kind, path) = sync_error_details(error);
    tracing::warn!(
        operation = operation,
        root = %root.display(),
        error_kind = error_kind,
        path = ?path,
        error = %error,
        "{}",
        message,
    );
}

/// The static category and the path (when named) for a sync failure.
fn sync_error_details(error: &SyncError) -> (&'static str, Option<String>) {
    match error {
        SyncError::Database(_) => ("database", None),
        SyncError::Reconcile(error) => reconcile_error_details(error),
        SyncError::Sweep(_) => ("sweep", None),
        SyncError::SweepBudgetExceeded { .. } => ("sweep_budget_exceeded", None),
    }
}

fn reconcile_error_details(error: &ReconcileError) -> (&'static str, Option<String>) {
    match error {
        ReconcileError::Discovery(error) => discovery_error_details(error),
        ReconcileError::Database(_) => ("database", None),
        ReconcileError::Hash(error) => match error {
            HashError::Read { path, .. } => ("hash_read", Some(path.display().to_string())),
            HashError::TooLarge { path, .. } => {
                ("hash_too_large", Some(path.display().to_string()))
            }
        },
        ReconcileError::Sample(error) => match error {
            SampleError::Read { path, .. } => ("sample_read", Some(path.display().to_string())),
            SampleError::TooLarge { path, .. } => {
                ("sample_too_large", Some(path.display().to_string()))
            }
        },
        ReconcileError::Revision(error) => revision_error_details(error),
        ReconcileError::BarrierCapExceeded { .. } => ("barrier_cap_exceeded", None),
        ReconcileError::TimeBudgetExceeded { path, .. } => (
            "reconcile_budget_exceeded",
            (!path.trim().is_empty()).then(|| path.trim().to_string()),
        ),
    }
}

fn discovery_error_details(error: &DiscoveryError) -> (&'static str, Option<String>) {
    match error {
        DiscoveryError::Walk(_) => ("discovery_walk", None),
        DiscoveryError::Read { path, .. } => ("discovery_read", Some(path.display().to_string())),
        DiscoveryError::NotRelative(path) => {
            ("discovery_not_relative", Some(path.display().to_string()))
        }
        DiscoveryError::NonUtf8Path(path) => {
            ("discovery_non_utf8_path", Some(path.display().to_string()))
        }
        DiscoveryError::TimeBudgetExceeded { .. } => ("discovery_budget_exceeded", None),
        DiscoveryError::IgnoreRules { path, .. } => (
            "discovery_ignore_rules",
            path.as_deref().map(|path| path.display().to_string()),
        ),
    }
}

fn revision_error_details(error: &RevisionError) -> (&'static str, Option<String>) {
    match error {
        RevisionError::Database(_) => ("revision_database", None),
        RevisionError::InvalidParserRun { path, .. } => {
            ("revision_invalid_parser_run", Some(path.clone()))
        }
        RevisionError::StaleBaseline { .. } => ("revision_stale_baseline", None),
    }
}
