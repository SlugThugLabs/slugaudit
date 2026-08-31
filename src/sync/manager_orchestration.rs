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
        sink.emit(ProgressEvent::Started {
            phase: "ensuring_current",
        });
        let (root, database_path) = project::resolve_project(Path::new(path))
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;

        let mut connection = match store::open_read_write(&database_path) {
            Ok(connection) => connection,
            Err(error) if error.is_corruption() => {
                tracing::warn!(
                    database_path = %database_path.display(),
                    error = %error,
                    "database is corrupt; discarding and re-publishing from scratch",
                );
                store::discard_corrupt_database(&database_path).map_err(|error| {
                    ErrorData::internal_error(
                        format!("discarding the corrupt project database: {error}"),
                        None,
                    )
                })?;
                return self.rebuild_and_finish(&root, database_path, sink);
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
                return Err(ErrorData::internal_error(
                    format!("opening the project database for sync: {error}"),
                    None,
                ));
            }
        };

        if let Err(metadata_error) = ensure_project_row(&mut connection, root.as_path()) {
            // Default recovery for a stale/incompatible derived database: any
            // evidence whose stored root or schema/contract version no longer
            // matches this build is disposable, so discard it and rebuild from
            // the current source rather than blocking the tool call. This
            // covers a repo that was moved, copied, or re-extracted from an
            // archive (root mismatch) and a database written by a newer or
            // differently-versioned SlugAudit (contract/schema mismatch).
            // Genuine safety rejections (symlink, network filesystem,
            // permission) and internal errors still surface as errors.
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
                // Drop the open handle before removing the file (SQLite
                // keeps the file open in WAL mode).
                drop(connection);
                store::discard_corrupt_database(&database_path).map_err(|discard| {
                    ErrorData::internal_error(
                        format!("discarding the stale project database: {discard}"),
                        None,
                    )
                })?;
                return self.rebuild_and_finish(&root, database_path, sink);
            }
            return Err(ErrorData::from(metadata_error));
        }

        let state = self.watch_manager.watch(root.as_path());
        // An ignore file may have changed since the last pass. Recompute
        // the watch scope (pruning or re-adding directory watches) and the
        // event-filtering rules now, before we decide what to reconcile —
        // otherwise a gitignored path could be indexed incrementally even
        // though a full publish would skip it.
        self.watch_manager.refresh_scope(root.as_path());
        let revision_id =
            self.sync_by_health(&root, &state, &mut connection, &database_path, sink)?;

        drop(connection);
        self.stamp_last_sync();
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
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        let synced = publish_from_scratch(root, database_path, sink)?;
        self.record_full_publish();
        self.stamp_last_sync();
        sink.emit(ProgressEvent::Completed {
            phase: "ensuring_current",
        });
        Ok(synced)
    }
}
