//! Project synchronization orchestration for [`super::SourceSyncManager`].
//!
//! This module owns the outer `ensure_current` workflow: resolving the
//! project, opening and recovering its derived database, and handing the
//! synchronized connection to the watcher-health module. Watcher state
//! transitions and incremental reconciliation live in `manager_health.rs`.

use super::super::manager_meta::{ProjectMetaError, ensure_project_row, publish_from_scratch};
use super::{SourceSyncManager, SyncedProject};
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::store;
use rmcp::ErrorData;
use std::path::{Path, PathBuf};

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
        let (root, database_path) = project::resolve_project(Path::new(path))
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;

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
                tracing::warn!(
                    database_path = %database_path.display(),
                    error = %error,
                    "database is corrupt or outdated; discarding and re-publishing from scratch",
                );
                store::discard_corrupt_database(database_path).map_err(|err| {
                    ErrorData::internal_error(
                        format!("discarding the corrupt or outdated project database: {err}"),
                        None,
                    )
                })?;
                self.rebuild_and_finish(root, database_path.to_path_buf(), start, sink)
                    .map(Err)
            }
            Err(error) => {
                tracing::warn!(
                    database_path = %database_path.display(),
                    error = %error,
                    "failed to open project database for sync",
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

    fn verify_metadata_or_rebuild(
        &self,
        root: &project::ProjectRoot,
        database_path: &Path,
        mut connection: rusqlite::Connection,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<Result<rusqlite::Connection, SyncedProject>, ErrorData> {
        if let Err(metadata_error) = ensure_project_row(&mut connection, root.as_path()) {
            let stale_reason = match &metadata_error {
                ProjectMetaError::RootMismatch { stored_root } => {
                    Some(format!("different project root ({stored_root})"))
                }
                ProjectMetaError::IncompatibleVersion {
                    contract_version,
                    schema_version,
                } => Some(format!(
                    "unsupported contract/schema version ({contract_version}/{schema_version})"
                )),
                ProjectMetaError::Other(_) => None,
            };
            if let Some(reason) = stale_reason {
                tracing::warn!(
                    database_path = %database_path.display(),
                    reason = %reason,
                    "project database is stale; discarding and rebuilding from source",
                );
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
            return Err(ErrorData::from(metadata_error));
        }
        Ok(Ok(connection))
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

        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
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
        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.stamp_last_sync(duration_ms);
        sink.emit(ProgressEvent::Completed {
            phase: "ensuring_current",
        });
        Ok(synced)
    }
}
