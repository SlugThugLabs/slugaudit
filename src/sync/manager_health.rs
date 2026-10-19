//! Watcher-health synchronization for [`super::SourceSyncManager`].
//!
//! This module owns the health state machine and the incremental barrier
//! reconcile. It is a child of `manager`, so it can coordinate the manager's
//! private counters and watcher state without exposing implementation fields.
// slugaudit-line-exception: approved-by=slugthug; reason=the watcher-health state machine — the dispatch, the per-branch paths, the shared event drain, and the post-sync revision read — must be read together because the health transitions and their failure semantics are the invariant; a sibling file would hide which arm owns which transition

use super::super::manager_meta::current_revision_id;
use super::super::reconcile::ReconcileOptions;
use super::super::sweep::SweepError;
use super::{SourceSyncManager, SyncError};
use crate::error_codes;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::watch::{WatchState, WatcherHealth};
use rmcp::ErrorData;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

impl SourceSyncManager {
    /// Dispatches synchronization according to watcher health. The health
    /// transitions remain together because each branch couples verification,
    /// event draining, state changes, counters, and progress signaling.
    ///
    /// # Errors
    ///
    /// Returns an error if the full publish, reconcile, stat sweep, or
    /// revision read fails.
    pub(super) fn sync_by_health(
        &self,
        root: &project::ProjectRoot,
        state: &WatchState,
        connection: &mut Connection,
        database_path: &Path,
        sink: &dyn ProgressSink,
    ) -> Result<String, ErrorData> {
        let health = state.health();
        match health {
            WatcherHealth::NeedsVerification | WatcherHealth::Desynced => {
                self.verify_and_drain(root, state, connection, sink)
            }
            WatcherHealth::Healthy | WatcherHealth::Partial => {
                self.healthy_path(root, state, connection, database_path, sink)
            }
            WatcherHealth::Unavailable => {
                tracing::info!(
                    root_id = %crate::util::redacted_path(root.as_path()),
                    "watcher unavailable; running full publish",
                );
                let report = super::super::manager_meta::publish_full(
                    connection,
                    root.as_path(),
                    sink,
                    "publish on Unavailable path failed",
                )?;
                self.record_full_publish();
                Ok(report.revision_id)
            }
        }
    }

    /// Full verification path (untrusted watcher): publish everything, then
    /// drain any events that arrived while the publish was walking.
    fn verify_and_drain(
        &self,
        root: &project::ProjectRoot,
        state: &WatchState,
        connection: &mut Connection,
        sink: &dyn ProgressSink,
    ) -> Result<String, ErrorData> {
        tracing::info!(
            health = ?state.health(),
            root_id = %crate::util::redacted_path(root.as_path()),
            "watcher untrusted; running full verification",
        );
        let report = super::super::manager_meta::publish_full(
            connection,
            root.as_path(),
            sink,
            "full publish failed",
        )?;
        // Drain any events that arrived during the full verification.
        // `publish` walks the filesystem and parses files, which takes
        // time — events can arrive while it runs. If we don't drain
        // them here, they'd wait until the next MCP call to be
        // reconciled, leaving the database stale in the interim.
        self.reconcile(root.as_path(), state, connection)
            .map_err(|error| {
                tracing::warn!(
                    root_id = %crate::util::redacted_path(root.as_path()),
                    error = %error,
                    "post-verification drain failed; events remain unreconciled",
                );
                let data = error_codes::budget_payload(error.budget_elapsed_ms());
                ErrorData::internal_error(
                    format!("draining events after verification: {error}"),
                    data,
                )
            })?;
        // A watcher that is active but whose scope walk stopped at a
        // documented limit is still trusted for the directories it watches;
        // the stat sweep covers the rest. Reporting `Healthy` would hide the
        // incomplete watch set, and `NeedsVerification` would force a full
        // publish on every call, so the verification ends `Partial` instead.
        let health = if self.watch_manager.scope_limited(root.as_path()) {
            WatcherHealth::Partial
        } else {
            WatcherHealth::Healthy
        };
        state.set_health(health);
        self.record_full_publish();
        Ok(report.revision_id)
    }

    /// Healthy path: walk exactly once, reconcile the watcher's events
    /// against that walk, apply the sweep's findings, and read the revision
    /// the operation just converged on.
    fn healthy_path(
        &self,
        root: &project::ProjectRoot,
        state: &WatchState,
        connection: &mut Connection,
        database_path: &Path,
        sink: &dyn ProgressSink,
    ) -> Result<String, ErrorData> {
        // An ignore file may have changed since the last pass: recompute
        // the scope and the event-filtering rules before the reconcile
        // reads them. A no-op when nothing changed.
        self.watch_manager.refresh_scope(root.as_path());
        // One deadline spans the whole Healthy operation — the sweep
        // walk, the event-driven reconcile, and applying the sweep's
        // findings — so no step silently starts a fresh clock.
        let options = ReconcileOptions::for_sync(
            root.as_path(),
            self.watch_manager.rules_for(root.as_path()),
        );
        // Walk exactly once: the report's `on_disk` serves the
        // event-driven reconcile below, and its candidates/deletions
        // are applied after it. A failed walk is as fatal as in the
        // sweep-driven reconcile — an unverified tree must not be
        // published against.
        let report = super::super::sweep_reconcile::sweep_tree(
            root.as_path(),
            connection,
            &options.deadline,
        )
        .map_err(|error| {
            super::super::sweep_reconcile::sweep_failed(root.as_path(), state, sink, error)
        })?;
        if state.has_unreconciled_events() {
            self.drain_events(root, state, connection, &options, &report.on_disk, sink)?;
        }
        // The sweep's findings, applied after the event-driven
        // reconcile so the observable order is unchanged — only the
        // walk moved earlier. Failure marks the watcher Desynced.
        super::super::sweep_reconcile::sweep_and_reconcile(
            root.as_path(),
            report,
            &options,
            connection,
            state,
            sink,
        )?;
        revision_after_sync(connection, database_path)
    }

    /// Drains the watcher's events under the operation's shared deadline.
    /// On failure the dirty sets are already cleared — the events are lost —
    /// so the watcher is marked `Desynced` and the next call re-verifies
    /// rather than silently serving stale evidence.
    fn drain_events(
        &self,
        root: &project::ProjectRoot,
        state: &WatchState,
        connection: &mut Connection,
        options: &ReconcileOptions,
        on_disk: &HashSet<String>,
        sink: &dyn ProgressSink,
    ) -> Result<(), ErrorData> {
        match reconcile_events(root.as_path(), state, connection, options, on_disk) {
            Ok(()) => {
                self.record_incremental_reconcile();
                Ok(())
            }
            Err(error) => {
                tracing::warn!(
                    root_id = %crate::util::redacted_path(root.as_path()),
                    error = %error,
                    "incremental reconcile failed; marking watcher Desynced so next call re-verifies",
                );
                state.set_health(WatcherHealth::Desynced);
                sink.emit(ProgressEvent::Completed {
                    phase: "ensuring_current",
                });
                let data = error_codes::budget_payload(error.budget_elapsed_ms());
                Err(ErrorData::internal_error(
                    format!("reconciling watcher events: {error}"),
                    data,
                ))
            }
        }
    }

    /// Reconciles unreconciled watcher events against the database using
    /// barrier synchronization: reconciles dirty/deleted paths, then checks
    /// if new events arrived during reconciliation and loops until the
    /// watcher sequence stabilizes. Only acknowledges events after the
    /// reconciliation succeeds.
    ///
    /// This is the entry point for a watcher that holds no sweep report of
    /// its own (the `NeedsVerification`/`Desynced` post-verification drain):
    /// it walks once to obtain `on_disk`. The `Healthy` path walks once up
    /// front and reuses that report via [`reconcile_events`], so it never
    /// picks up a second whole-tree walk here.
    ///
    /// # Errors
    ///
    /// Returns an error if reading a dirty file, querying the database, or
    /// committing the revision fails. On failure, the watcher health should
    /// be set to `NeedsVerification` or `Desynced` so the next call re-verifies.
    pub fn reconcile(
        &self,
        root: &Path,
        state: &WatchState,
        connection: &mut Connection,
    ) -> Result<(), SyncError> {
        // An ignore file may have changed since the last pass: recompute
        // the watch scope and the event-filtering rules before deciding
        // what to reconcile. A no-op when nothing changed.
        self.watch_manager.refresh_scope(root);
        // One deadline for the whole barrier operation — every iteration
        // and every per-path reconcile inside it — so a pathological
        // event producer can't stall a tool call indefinitely. The
        // project's ignore rules make the incremental path index exactly
        // what a full publish would index.
        let options = ReconcileOptions::for_sync(root, self.watch_manager.rules_for(root));
        // Edge resolution must treat every path that exists as existing —
        // not just indexed rows. A file sampling skipped (oversize,
        // unreadable) has no `files` row, so resolving its importers
        // against indexed rows alone would write `Unresolved`, which reads
        // as dead code. This path has no report in hand, so it walks once;
        // its failure is as fatal here as in the sweep-driven reconcile:
        // an unverified tree must not be published against.
        let report = super::super::sweep::sweep(connection, root, &options.deadline)
            .map_err(sweep_error_to_sync)?;
        reconcile_events(root, state, connection, &options, &report.on_disk)
    }
}

/// Reads the revision id the sync operation just converged on. Every field
/// is present or the call errors with a named reason — a successful sync
/// that cannot name its revision is a bug an operator must be able to see.
fn revision_after_sync(connection: &Connection, database_path: &Path) -> Result<String, ErrorData> {
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
/// is read here — at the point this step publishes — so the CAS baseline is
/// never shared with another publishing step (the sweep apply reads its own
/// immediately before it publishes).
///
/// # Errors
///
/// Returns an error if the barrier loop or any per-path reconcile fails.
fn reconcile_events(
    root: &Path,
    state: &WatchState,
    connection: &mut Connection,
    options: &ReconcileOptions,
    on_disk: &HashSet<String>,
) -> Result<(), SyncError> {
    let expected_current = current_revision_id(connection)?;
    let started = std::time::Instant::now();
    // Accumulated across every barrier iteration so the log line below
    // reports the total skip count, not just the last pass's.
    let mut skipped_total = 0usize;

    super::super::reconcile::sync_with_barrier_with_deadline(
        state,
        &options.deadline,
        |dirty, deleted| {
            let report = super::super::reconcile::reconcile_dirty_paths_with_deadline(
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
fn sweep_error_to_sync(error: SweepError) -> SyncError {
    match error {
        SweepError::TimeBudgetExceeded { elapsed_ms } => {
            SyncError::SweepBudgetExceeded { elapsed_ms }
        }
        other => SyncError::Sweep(other.to_string()),
    }
}
