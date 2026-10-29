//! The `Healthy`/`Partial` branch of the watcher-health dispatch: the
//! skip-or-sweep decision, the single whole-tree walk, the event drain, and
//! the apply. Split from `manager_health.rs` (a child of `manager` like it)
//! to stay under the source-size rule.
use super::super::manager_health_support::{
    limits_error, log_sync_failure, reconcile_events, revision_after_sync,
};
use super::super::reconcile::ReconcileOptions;
use super::SourceSyncManager;
use crate::error_codes;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::watch::{WatchState, WatcherHealth};
use rmcp::ErrorData;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

impl SourceSyncManager {
    /// Healthy path: walk exactly once, reconcile the watcher's events
    /// against that walk, apply the sweep's findings, and read the revision
    /// the operation just converged on.
    pub(super) fn healthy_path(
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
        self.refresh_watch_scope_for_tool(root.as_path())?;
        // Healthy, quiet watcher, and the last full sweep is recent and used
        // the same ignore rules: nothing the watcher can report is pending,
        // so the whole-tree stat walk is skipped. The decision reads health
        // and the dirty queue under one lock; an event landing after it is
        // reconciled by the next call. The backstop in `full_sweep_gate`
        // bounds how long an event the watcher never reports can go unseen.
        let rules = self.watch_manager.rules_for(root.as_path());
        if state.is_healthy_and_quiet()
            && self
                .full_sweep_gate
                .may_skip(root.as_path(), rules.as_ref())
        {
            tracing::debug!(
                root_id = %crate::util::redacted_path(root.as_path()),
                "watcher healthy and quiet; skipping the full stat sweep",
            );
            return revision_after_sync(connection, database_path);
        }
        let sweep_started = self.full_sweep_gate.now();
        // One deadline spans the whole Healthy operation — the sweep
        // walk, the event-driven reconcile, and applying the sweep's
        // findings — so no step silently starts a fresh clock.
        let options =
            ReconcileOptions::for_sync(root.as_path(), rules.clone()).map_err(limits_error)?;
        // Walk exactly once: the report's `on_disk` serves the
        // event-driven reconcile below, and its candidates/deletions
        // are applied after it. A failed walk is as fatal as in the
        // sweep-driven reconcile — an unverified tree must not be
        // published against.
        self.full_sweep_gate.count_healthy_walk();
        let report = super::super::sweep_reconcile::sweep_tree(
            root.as_path(),
            connection,
            &options.deadline,
        )
        .map_err(|error| {
            self.full_sweep_gate.invalidate(root.as_path());
            super::super::sweep_reconcile::sweep_failed(root.as_path(), state, sink, error)
        })?;
        // The incremental walk is the only place a non-UTF-8 add or remove
        // can be seen between full publishes; persist it before the report
        // can read the count.
        let persisted = super::super::publish::replace_unrepresentable_paths(
            connection,
            &report.unrepresentable,
        );
        persisted.map_err(|error| {
            ErrorData::internal_error(format!("recording unrepresentable paths: {error}"), None)
        })?;
        if state.has_unreconciled_events() {
            self.drain_events(root, state, connection, &options, &report.on_disk, sink)
                .inspect_err(|_| self.full_sweep_gate.invalidate(root.as_path()))?;
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
        )
        .inspect_err(|_| self.full_sweep_gate.invalidate(root.as_path()))?;
        // Walk, event reconcile, and apply all succeeded: only now does the
        // backstop window restart. Any earlier failure returned above
        // without recording.
        self.full_sweep_gate
            .record(root.as_path(), sweep_started, rules);
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
                log_sync_failure(
                    "reconcile_drain",
                    root.as_path(),
                    "incremental reconcile failed; marking watcher Desynced so next call re-verifies",
                    &error,
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
}
