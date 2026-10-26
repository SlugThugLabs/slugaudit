// slugaudit-line-exception: approved-by=slugthug; reason=Recombined manager orchestration workflow and database recovery diagnostics into cohesive module.
//! Project synchronization orchestration for [`super::SourceSyncManager`].
//!
//! This module owns the outer `ensure_current` workflow: resolving the
//! project, opening and recovering its derived database, and handing the
//! synchronized connection to the watcher-health module. Watcher state
//! transitions and incremental reconciliation live in `manager_health.rs`.

use super::super::manager_meta::{
    ProjectMetaError, ensure_project_row, publish_from_scratch, purge_prior_session_findings,
};
use super::{SourceSyncManager, SyncedProject};
use crate::error_codes::{self, ErrorCode};
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::store::{self, StoreError};
use rmcp::ErrorData;
use std::path::{Path, PathBuf};

/// One database recovery event: a database that failed to open, or one whose
/// metadata proves it belongs to another project root or another contract.
enum DatabaseRecovery<'a> {
    OpenFailed {
        error: &'a StoreError,
        message: &'static str,
    },
    RootMismatch {
        stored_root: &'a str,
    },
    IncompatibleVersion {
        contract_version: i64,
        schema_version: i64,
    },
}

fn log_database_recovery(database_path: &Path, recovery: DatabaseRecovery<'_>) {
    match recovery {
        DatabaseRecovery::OpenFailed { error, message } => tracing::warn!(
            operation = "open_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = store_error_kind(error),
            "{}",
            message,
        ),
        DatabaseRecovery::RootMismatch { stored_root } => tracing::warn!(
            operation = "verify_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = "root_mismatch",
            stored_root_id = %crate::util::redacted_path(Path::new(stored_root)),
            "project database is stale; discarding and rebuilding from source",
        ),
        DatabaseRecovery::IncompatibleVersion {
            contract_version,
            schema_version,
        } => tracing::warn!(
            operation = "verify_project_database",
            database_id = %crate::util::redacted_path(database_path),
            error_kind = "incompatible_version",
            contract_version,
            schema_version,
            "project database is stale; discarding and rebuilding from source",
        ),
    }
}

fn store_error_kind(error: &StoreError) -> &'static str {
    match error {
        StoreError::Open(_) => "open",
        StoreError::Configure(_) => "configure",
        StoreError::Busy(_) => "busy",
        StoreError::Migration(_) => "migration",
        StoreError::Symlink => "symlink",
        StoreError::Inspect(_) => "inspect",
        StoreError::WalUnavailable { .. } => "wal_unavailable",
        StoreError::Permissions(_) => "permissions",
        StoreError::NetworkFilesystem => "network_filesystem",
        StoreError::NetworkFilesystemCheck(_) => "network_filesystem_check",
    }
}

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
        match store::open_read_write(database_path) {
            Ok(connection) => Ok(Ok(connection)),
            Err(error) if error.is_corruption() => {
                log_database_recovery(
                    database_path,
                    DatabaseRecovery::OpenFailed {
                        error: &error,
                        message: "database is corrupt or outdated; \
                                  discarding and re-publishing from scratch",
                    },
                );
                // Re-checked under the database lock: a second session may
                // have repaired the file since this open failed.
                let outcome = store::discard_if_still_unusable(database_path).map_err(|err| {
                    ErrorData::internal_error(
                        format!("discarding the corrupt or outdated project database: {err}"),
                        None,
                    )
                })?;
                if outcome == store::DiscardOutcome::AlreadyCurrent {
                    return store::open_read_write(database_path)
                        .map(Ok)
                        .map_err(|error| {
                            ErrorData::internal_error(
                                format!("opening the project database for sync: {error}"),
                                None,
                            )
                        });
                }
                self.rebuild_and_finish(root, database_path.to_path_buf(), start, sink)
                    .map(Err)
            }
            Err(error) => {
                log_database_recovery(
                    database_path,
                    DatabaseRecovery::OpenFailed {
                        error: &error,
                        message: "failed to open project database for sync",
                    },
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
        self.purge_prior_session_findings_once(database_path, &mut connection)?;
        if let Err(metadata_error) = ensure_project_row(&mut connection, root.as_path()) {
            match metadata_error {
                ProjectMetaError::RootMismatch { stored_root } => {
                    log_database_recovery(
                        database_path,
                        DatabaseRecovery::RootMismatch {
                            stored_root: &stored_root,
                        },
                    );
                }
                ProjectMetaError::IncompatibleVersion {
                    contract_version,
                    schema_version,
                } => {
                    log_database_recovery(
                        database_path,
                        DatabaseRecovery::IncompatibleVersion {
                            contract_version,
                            schema_version,
                        },
                    );
                }
                other @ ProjectMetaError::Other(_) => return Err(ErrorData::from(other)),
            }
            drop(connection);
            store::discard_corrupt_database(database_path).map_err(|discard| {
                ErrorData::internal_error(
                    format!("discarding the stale project database: {discard}"),
                    None,
                )
            })?;
            return self
                .rebuild_and_finish(root, database_path.to_path_buf(), start, sink)
                .map(Err);
        }
        Ok(Ok(connection))
    }

    /// Purges findings left by other agent sessions exactly once per
    /// project database in this process/session.
    ///
    /// `ensure_current` runs on every tool call, but a live MCP process may
    /// be only one of several sessions sharing the project database. Purging
    /// on every call would delete another live session's findings
    /// mid-session, and no response carries that deletion. The guard is
    /// keyed by database path (one manager can serve several projects) and
    /// records the session UUID the purge ran for, so the documented
    /// cross-session defense still fires on this process's first sync of
    /// each project and a test-only session change re-arms it.
    ///
    /// A failed purge aborts the call with its own error and leaves the
    /// guard unarmed, so the next sync retries the session-start
    /// transition instead of serving a database that still holds prior
    /// sessions' findings.
    fn purge_prior_session_findings_once(
        &self,
        database_path: &Path,
        connection: &mut rusqlite::Connection,
    ) -> Result<(), ErrorData> {
        let current_session_id = crate::util::session_id();
        let mut purged =
            crate::util::lock_or_recover(&self.session_findings_purged, "session_findings_purged");
        if purged.get(database_path).copied() == Some(current_session_id) {
            return Ok(());
        }
        purge_prior_session_findings(connection, current_session_id)?;
        purged.insert(database_path.to_path_buf(), current_session_id);
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
        let state = self.watch_manager.watch(root.as_path());
        self.watch_manager.refresh_scope(root.as_path());
        let revision_id = self.sync_by_health(root, &state, connection, &database_path, sink)?;

        let duration_ms = elapsed_ms(start);
        self.stamp_last_sync(duration_ms);
        tracing::debug!(
            revision_id = %revision_id,
            root_id = %crate::util::redacted_path(root.as_path()),
            "ensure_current completed",
        );

        sink.emit(ProgressEvent::Completed {
            phase: "ensuring_current",
        });
        Ok(SyncedProject {
            database_path,
            revision_id,
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
    ) -> Result<SyncedProject, ErrorData> {
        let synced = publish_from_scratch(root, database_path, sink)?;
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
