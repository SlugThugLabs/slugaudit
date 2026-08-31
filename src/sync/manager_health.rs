//! Watcher-health synchronization for [`super::SourceSyncManager`].
//!
//! This module owns the health state machine and the incremental barrier
//! reconcile. It is a child of `manager`, so it can coordinate the manager's
//! private counters and watcher state without exposing implementation fields.

use super::super::manager_meta::current_revision_id;
use super::super::reconcile::ReconcileOptions;
use super::{SourceSyncManager, SyncError};
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::watch::{WatchState, WatcherHealth};
use rmcp::ErrorData;
use rusqlite::Connection;
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
                tracing::info!(
                    ?health,
                    root = %root.as_path().display(),
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
                            root = %root.as_path().display(),
                            error = %error,
                            "post-verification drain failed; events remain unreconciled",
                        );
                        ErrorData::internal_error(
                            format!("draining events after verification: {error}"),
                            None,
                        )
                    })?;
                state.set_health(WatcherHealth::Healthy);
                self.record_full_publish();
                Ok(report.revision_id)
            }
            WatcherHealth::Healthy => {
                if state.has_unreconciled_events() {
                    match self.reconcile(root.as_path(), state, connection) {
                        Ok(()) => self.record_incremental_reconcile(),
                        Err(error) => {
                            // `snapshot_dirty` cleared the dirty sets, but
                            // reconciliation failed — the events are lost. Mark
                            // the watcher untrusted so the next call does a full
                            // verification rather than silently serving stale
                            // evidence.
                            tracing::warn!(
                                root = %root.as_path().display(),
                                error = %error,
                                "incremental reconcile failed; marking watcher Desynced so next call re-verifies",
                            );
                            state.set_health(WatcherHealth::Desynced);
                            sink.emit(ProgressEvent::Completed {
                                phase: "ensuring_current",
                            });
                            return Err(ErrorData::internal_error(
                                format!("reconciling watcher events: {error}"),
                                None,
                            ));
                        }
                    }
                }
                // Stat sweep: the watcher-independent freshness backstop — a
                // change the watcher dropped or never saw is still found and
                // reconciled before this call serves evidence. Failure marks
                // the watcher Desynced so the next call re-verifies.
                super::super::sweep_reconcile::sweep_and_reconcile(
                    root.as_path(),
                    self.watch_manager.rules_for(root.as_path()),
                    connection,
                    state,
                    sink,
                )?;

                Ok(current_revision_id(connection)
                    .map_err(|error| {
                        tracing::warn!(
                            database_path = %database_path.display(),
                            error = %error,
                            "failed to read the current revision",
                        );
                        ErrorData::internal_error(
                            format!("reading the current revision: {error}"),
                            None,
                        )
                    })?
                    .ok_or_else(|| {
                        tracing::warn!(
                            database_path = %database_path.display(),
                            "no current revision found after sync",
                        );
                        ErrorData::internal_error(
                            "no current revision found after sync — this is unexpected; \
                         try disabling and re-enabling the project",
                            None,
                        )
                    })?)
            }
            WatcherHealth::Unavailable => {
                tracing::info!(
                    root = %root.as_path().display(),
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

    /// Reconciles unreconciled watcher events against the database using
    /// barrier synchronization: reconciles dirty/deleted paths, then checks
    /// if new events arrived during reconciliation and loops until the
    /// watcher sequence stabilizes. Only acknowledges events after the
    /// reconciliation succeeds.
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
        let expected_current = current_revision_id(connection)?;
        // One deadline for the whole barrier operation — every iteration
        // and every per-path reconcile inside it — so a pathological
        // event producer can't stall a tool call indefinitely. The
        // project's ignore rules make the incremental path index exactly
        // what a full publish would index.
        let options = ReconcileOptions::for_sync(self.watch_manager.rules_for(root));
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
                    &options,
                )?;
                skipped_total += report.skipped;
                Ok(())
            },
        )?;

        tracing::debug!(
            root = %root.display(),
            elapsed_ms = started.elapsed().as_millis(),
            skipped = skipped_total,
            "incremental reconcile phase complete",
        );
        Ok(())
    }
}
