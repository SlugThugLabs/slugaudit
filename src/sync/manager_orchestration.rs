// slugaudit-line-exception: approved-by=slugthug; reason=Recombined manager orchestration workflow and database recovery diagnostics into cohesive module.
//! Project synchronization orchestration for [`super::SourceSyncManager`].
//!
//! This module owns the outer `ensure_current` workflow: resolving the
//! project, opening and recovering its derived database, and handing the
//! synchronized connection to the watcher-health module. Watcher state
//! transitions and incremental reconciliation live in `manager_health.rs`.

use super::super::discard_notice::{DatabaseDiscard, count_findings, count_findings_at};
use super::super::manager_meta::{
    ProjectMetaError, ensure_project_row, mark_prior_session_findings_stale, publish_from_scratch,
};
use super::super::recovery_log::{DatabaseRecovery, log_database_recovery};
use super::{SourceSyncManager, SyncedProject};
use crate::error_codes::{self, ErrorCode};
use crate::model::process_limits;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::store;
use rmcp::ErrorData;
use std::path::{Path, PathBuf};

fn elapsed_ms(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "sync duration exceeds u64 milliseconds; stamp saturates");
        u64::MAX
    })
}

impl SourceSyncManager {
    /// Ensures the project containing `path` is fully synchronized and
    /// returns a handle to its current revision. Uses the filesystem
    /// watcher to avoid full publishes when possible:
    ///
    /// - `NeedsVerification` / `Desynced`: full publish, then health → Healthy.
    /// - `Healthy` with unreconciled events: incremental reconcile.
    /// - `Healthy` without unreconciled events: returns the current revision.
    /// - `Unavailable`: full publish.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` isn't inside an active project, or if
    /// sync itself fails.
    pub fn ensure_current(
        &self,
        path: &str,
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        let start = std::time::Instant::now();
        sink.emit(ProgressEvent::Started {
            phase: "ensuring_current",
        });
        let (root, database_path) =
            project::resolve_project(Path::new(path)).map_err(map_activation_error)?;
        // Test-only seam: the window in which `project_control off` can remove
        // the database after the activation check and before it is opened.
        // Keyed by the database path, not the root, because the publish hooks
        // already use the root as their key.
        super::super::race_hook::fire(&database_path);

        let connection = match self.open_or_rebuild(&root, &database_path, start, sink)? {
            Ok(conn) => conn,
            Err(synced) => return Ok(synced),
        };

        let mut connection = match self.verify_metadata_or_rebuild(
            &root,
            &database_path,
            connection,
            start,
            sink,
        )? {
            Ok(conn) => conn,
            Err(synced) => return Ok(synced),
        };

        self.reconcile_and_finish(&root, database_path, &mut connection, start, sink)
    }

    fn open_or_rebuild(
        &self,
        root: &project::ProjectRoot,
        database_path: &Path,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<Result<rusqlite::Connection, SyncedProject>, ErrorData> {
        // Never creates the file: a project deactivated since the check in
        // `resolve_project` must stay deactivated. Only the rebuild below
        // (inside a project that is still active) recreates it.
        match store::open_existing_read_write(database_path) {
            Ok(connection) => Ok(Ok(connection)),
            Err(store::StoreError::Missing) => {
                sink.emit(ProgressEvent::Completed {
                    phase: "ensuring_current",
                });
                Err(map_activation_error(project::ActivationError::NotActive))
            }
            Err(error) if error.is_corruption() => {
                let (notice, count_error) = DatabaseDiscard::assess(
                    count_findings_at(database_path),
                    format!("the project database is corrupt or outdated: {error}"),
                );
                log_database_recovery(
                    database_path,
                    DatabaseRecovery::OpenCorrupt {
                        error: &error,
                        discard: &notice,
                        count_error: count_error.as_ref(),
                    },
                );
                // Test-only seam: `project_control off` removing the corrupt
                // file between the failed open and the discard.
                super::super::race_hook::fire(discard_hook_key(database_path));
                // Re-checked under the database lock: a second session may
                // have repaired the file since this open failed.
                let outcome = store::discard_if_still_unusable(database_path).map_err(|err| {
                    ErrorData::internal_error(
                        format!("discarding the corrupt or outdated project database: {err}"),
                        None,
                    )
                })?;
                if outcome == store::DiscardOutcome::Gone {
                    sink.emit(ProgressEvent::Completed {
                        phase: "ensuring_current",
                    });
                    return Err(map_activation_error(project::ActivationError::NotActive));
                }
                if outcome == store::DiscardOutcome::AlreadyCurrent {
                    return store::open_existing_read_write(database_path)
                        .map(Ok)
                        .map_err(|error| match error {
                            store::StoreError::Missing => {
                                map_activation_error(project::ActivationError::NotActive)
                            }
                            error => ErrorData::internal_error(
                                format!("opening the project database for sync: {error}"),
                                None,
                            ),
                        });
                }
                self.rebuild_and_finish(root, database_path.to_path_buf(), start, sink, notice)
                    .map(Err)
            }
            Err(error) => {
                log_database_recovery(
                    database_path,
                    DatabaseRecovery::OpenFailed { error: &error },
                );
                sink.emit(ProgressEvent::Completed {
                    phase: "ensuring_current",
                });
                Err(ErrorData::internal_error(
                    format!("opening the project database for sync: {error}"),
                    None,
                ))
            }
        }
    }

    pub(super) fn verify_metadata_or_rebuild(
        &self,
        root: &project::ProjectRoot,
        database_path: &Path,
        mut connection: rusqlite::Connection,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<Result<rusqlite::Connection, SyncedProject>, ErrorData> {
        self.mark_prior_sessions_stale_once(database_path, &mut connection)?;
        if let Err(metadata_error) = ensure_project_row(&mut connection, root.as_path()) {
            let cause = match &metadata_error {
                ProjectMetaError::RootMismatch { .. } => {
                    "the project database was built for a different project root".to_owned()
                }
                ProjectMetaError::IncompatibleVersion {
                    contract_version,
                    schema_version,
                } => format!(
                    "the project database has an incompatible version \
                     (contract_version {contract_version}, schema_version {schema_version})"
                ),
                ProjectMetaError::Other(_) => {
                    return Err(ErrorData::from(metadata_error));
                }
            };
            let (notice, count_error) = DatabaseDiscard::assess(count_findings(&connection), cause);
            let count_error = count_error.as_ref();
            match &metadata_error {
                ProjectMetaError::RootMismatch { stored_root } => log_database_recovery(
                    database_path,
                    DatabaseRecovery::RootMismatch {
                        stored_root,
                        discard: &notice,
                        count_error,
                    },
                ),
                ProjectMetaError::IncompatibleVersion {
                    contract_version,
                    schema_version,
                } => log_database_recovery(
                    database_path,
                    DatabaseRecovery::IncompatibleVersion {
                        contract_version: *contract_version,
                        schema_version: *schema_version,
                        discard: &notice,
                        count_error,
                    },
                ),
                ProjectMetaError::Other(_) => {}
            }
            drop(connection);
            // Test-only seam: `off` removing the database between the
            // metadata mismatch and the discard.
            super::super::race_hook::fire(discard_hook_key(database_path));
            // A database `off` already removed must stay removed.
            match database_path.symlink_metadata() {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    sink.emit(ProgressEvent::Completed {
                        phase: "ensuring_current",
                    });
                    return Err(map_activation_error(project::ActivationError::NotActive));
                }
                Err(error) => {
                    return Err(ErrorData::internal_error(
                        format!("inspecting the stale project database: {error}"),
                        None,
                    ));
                }
            }
            store::discard_corrupt_database(database_path).map_err(|discard| {
                ErrorData::internal_error(
                    format!("discarding the stale project database: {discard}"),
                    None,
                )
            })?;
            return self
                .rebuild_and_finish(root, database_path.to_path_buf(), start, sink, notice)
                .map(Err);
        }
        Ok(Ok(connection))
    }

    /// Marks findings left by other agent sessions `stale` exactly once per
    /// project database in this process/session. Nothing is deleted.
    ///
    /// `ensure_current` runs on every tool call, but a live MCP process may
    /// be only one of several sessions sharing the project database. Marking
    /// on every call would re-stale a live peer's findings as it writes them.
    /// The guard is keyed by database path (one manager can serve several
    /// projects) and records the session UUID the marking ran for, so a
    /// test-only session change re-arms it. The number of rows marked is
    /// logged once at info.
    ///
    /// A failed update aborts the call with its own error and leaves the
    /// guard unarmed, so the next sync retries the session-start transition.
    fn mark_prior_sessions_stale_once(
        &self,
        database_path: &Path,
        connection: &mut rusqlite::Connection,
    ) -> Result<(), ErrorData> {
        let current_session_id = crate::util::session_id();
        let mut marked =
            crate::util::lock_or_recover(&self.prior_sessions_staled, "prior_sessions_staled");
        if marked.get(database_path).copied() == Some(current_session_id) {
            return Ok(());
        }
        let staled = mark_prior_session_findings_stale(connection, current_session_id)?;
        tracing::info!(
            staled,
            "prior-session findings marked stale at session start"
        );
        marked.insert(database_path.to_path_buf(), current_session_id);
        Ok(())
    }

    fn reconcile_and_finish(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
        connection: &mut rusqlite::Connection,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        let limits = process_limits().map_err(map_process_limits_error)?;
        let state = self.watch_manager.watch(root.as_path(), limits);
        self.watch_manager.refresh_scope(root.as_path(), limits);
        let revision_id = self.sync_by_health(root, &state, connection, &database_path, sink)?;

        let duration_ms = elapsed_ms(start);
        self.stamp_last_sync(duration_ms);
        tracing::debug!(
            revision_id = %revision_id,
            root = %root.as_path().display(),
            "ensure_current completed",
        );

        sink.emit(ProgressEvent::Completed {
            phase: "ensuring_current",
        });
        Ok(SyncedProject {
            database_path,
            revision_id,
            discard: None,
        })
    }

    /// Shared tail for the corruption and stale-database recovery paths:
    /// rebuilds the database from scratch, records the full publish,
    /// stamps the sync time, and emits the completion signal. Extracted so
    /// the recovery paths cannot drift apart.
    fn rebuild_and_finish(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
        discard: DatabaseDiscard,
    ) -> Result<SyncedProject, ErrorData> {
        let mut synced = publish_from_scratch(root, database_path, sink)?;
        synced.discard = Some(discard);
        self.record_full_publish();
        let duration_ms = elapsed_ms(start);
        self.stamp_last_sync(duration_ms);
        sink.emit(ProgressEvent::Completed {
            phase: "ensuring_current",
        });
        Ok(synced)
    }
}

/// Every state-bearing tool funnels through `ensure_current`, so this is
/// the one place the "no project here" case can be shaped: a path with no
/// activation marker is not an opaque failure of whichever tool was called
/// — it is a project that was never enabled, and the response carries
/// `project_not_enabled` plus the exact remedy (`project_control` with
/// `action: "on"` and the path argument). Any other resolution failure — an
/// unresolvable path, a symlinked activation directory — keeps its plain
/// message and no code, because `project_control` cannot fix it.
pub(crate) fn map_activation_error(error: project::ActivationError) -> ErrorData {
    let data = if matches!(error, project::ActivationError::NotActive) {
        error_codes::payload(ErrorCode::ProjectNotEnabled)
    } else {
        None
    };
    ErrorData::invalid_params(error.to_string(), data)
}

impl SourceSyncManager {
    /// [`SourceSyncManager::refresh_watch_scope`] with the error shaped for a
    /// tool call.
    pub(super) fn refresh_watch_scope_for_tool(&self, root: &Path) -> Result<(), ErrorData> {
        self.refresh_watch_scope(root)
            .map_err(map_process_limits_error)
    }
}

/// The tool error for process limits that were never installed and whose
/// `SLUGAUDIT_*` environment is invalid (a library embedder; the shipped
/// binary validates before serving). Logged once here, with the source error.
pub(crate) fn map_process_limits_error(error: crate::model::EnvConfigError) -> ErrorData {
    tracing::error!(%error, "process limits environment is invalid");
    ErrorData::internal_error(
        format!("the SLUGAUDIT_* environment is invalid: {error}; fix it and restart the server"),
        None,
    )
}

/// Race-hook key for the window between a failed open or metadata check and
/// the discard that follows it: the activation directory, which exists until
/// `off` and is distinct from the database-path key of the window before the
/// open. A path that does not exist would log a canonicalization warning on
/// every call, so the key must name something present.
fn discard_hook_key(database_path: &Path) -> &Path {
    database_path.parent().unwrap_or(database_path)
}
