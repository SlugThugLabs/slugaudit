// slugaudit-line-exception: approved-by=slugthug; reason=the sync recovery workflow: ensure_current plus the open, verify and publish steps that each discard and rebuild a bad database, with their error shaping
//! Project synchronization orchestration for [`super::SourceSyncManager`].
//!
//! This module owns the `ensure_current` workflow: resolving the project,
//! the once-per-process start of its session findings, opening and
//! recovering its derived database, and one full source scan per call. The scan itself
//! (discovery, hashing, diff, publish) lives under `publish`; this module
//! owns the surrounding recovery and error shaping.

use super::super::discard_notice::DatabaseDiscard;
use super::super::manager_meta::{
    ProjectMetaError, ensure_project_row, map_publish_error, publish_from_scratch,
    revision_after_sync,
};
use super::super::publish::{PublishReport, publish};
use super::super::recovery_log::{DatabaseRecovery, log_database_recovery};
use super::{SourceSyncManager, SyncedProject};
use crate::error_codes::{self, ErrorCode};
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::store;
use rmcp::ErrorData;
use std::path::{Path, PathBuf};

/// A bad database to delete before rebuilding, with the notice that tells
/// the model why and the wording of the discard failure message.
struct Discard {
    notice: DatabaseDiscard,
    /// Fills `discarding the <what> project database: ...`.
    what: &'static str,
}

impl SourceSyncManager {
    // Owner decision (2026-10-31): the database only matters if it is exactly the same as the code base, and hashing every file on every call is simpler than trusting any shortcut.
    /// Ensures the project containing `path` is fully synchronized and
    /// returns a handle to its current revision.
    ///
    /// Every call walks the project and hashes every indexable file; only
    /// files whose content differs from the stored row are re-parsed and
    /// rewritten. The stored-content check also proves each row still
    /// matches its own hash, so a database edited outside this process is
    /// repaired from the source before the call returns.
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
            .map_err(|error| map_activation_error(&error))?;
        // The first touch of a project in this process starts its session
        // findings fresh; every later call, and any rebuild of the project
        // database below, leaves the session findings file alone.
        project::begin_process_session(&root).map_err(|error| map_activation_error(&error))?;

        let connection = match self.open_or_rebuild(&root, &database_path, start, sink)? {
            Ok(conn) => conn,
            Err(synced) => return Ok(synced),
        };

        let connection = match self.verify_metadata_or_rebuild(
            &root,
            &database_path,
            connection,
            start,
            sink,
        )? {
            Ok(conn) => conn,
            Err(synced) => return Ok(synced),
        };

        let (connection, report) =
            match self.publish_or_rebuild(&root, &database_path, connection, start, sink)? {
                Ok(published) => published,
                Err(synced) => return Ok(synced),
            };
        let revision_id = revision_after_sync(&connection, &database_path)?;

        let duration_ms = crate::clock::saturating_millis(start.elapsed());
        self.stamp_last_sync(duration_ms)?;
        tracing::debug!(
            revision_id = %revision_id,
            root = %root.as_path().display(),
            added = report.added,
            modified = report.modified,
            deleted = report.deleted,
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

    fn open_or_rebuild(
        &self,
        root: &project::ProjectRoot,
        database_path: &Path,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<Result<rusqlite::Connection, SyncedProject>, ErrorData> {
        // Never creates the file here: the rebuild below does, and it fails
        // cleanly if `.planning/slugaudit` itself is gone (see
        // `publish_from_scratch`), because only `project_control on` creates
        // that directory.
        match store::open_existing_read_write(database_path) {
            Ok(connection) => Ok(Ok(connection)),
            // The database is scratch: a missing one is rebuilt, with no
            // discard notice because no earlier contents are known.
            Err(store::StoreError::Missing) => self
                .rebuild_and_finish(root, database_path.to_path_buf(), start, sink, None)
                .map(Err),
            Err(error) if error.is_corruption() => {
                let notice = DatabaseDiscard {
                    cause: format!("the project database is corrupt or outdated: {error}"),
                };
                log_database_recovery(
                    database_path,
                    &DatabaseRecovery::OpenCorrupt { error: &error },
                );
                self.rebuild_and_finish(
                    root,
                    database_path.to_path_buf(),
                    start,
                    sink,
                    Some(Discard {
                        notice,
                        what: "corrupt or outdated",
                    }),
                )
                .map(Err)
            }
            Err(error) => {
                log_database_recovery(
                    database_path,
                    &DatabaseRecovery::OpenFailed { error: &error },
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

    /// Runs the one full publish. Corruption found mid-publish discards the
    /// database and rebuilds it (`Err(SyncedProject)` carries that result);
    /// any other publish failure ends the call with the mapped error.
    fn publish_or_rebuild(
        &self,
        root: &project::ProjectRoot,
        database_path: &Path,
        mut connection: rusqlite::Connection,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<Result<(rusqlite::Connection, PublishReport), SyncedProject>, ErrorData> {
        match publish(
            &mut connection,
            root.as_path(),
            crate::parse::PACK_VERSION,
            sink,
        ) {
            Ok(report) => Ok(Ok((connection, report))),
            Err(error) if error.is_corruption() => {
                let notice = DatabaseDiscard {
                    cause: format!("the project database became corrupt during sync: {error}"),
                };
                log_database_recovery(
                    database_path,
                    &DatabaseRecovery::PublishCorrupt { error: &error },
                );
                drop(connection);
                self.rebuild_and_finish(
                    root,
                    database_path.to_path_buf(),
                    start,
                    sink,
                    Some(Discard {
                        notice,
                        what: "corrupt",
                    }),
                )
                .map(Err)
            }
            Err(error) => {
                sink.emit(ProgressEvent::Completed {
                    phase: "ensuring_current",
                });
                Err(map_publish_error(root.as_path(), &error, "publish failed"))
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
            let notice = DatabaseDiscard { cause };
            match &metadata_error {
                ProjectMetaError::RootMismatch { stored_root } => log_database_recovery(
                    database_path,
                    &DatabaseRecovery::RootMismatch { stored_root },
                ),
                ProjectMetaError::IncompatibleVersion {
                    contract_version,
                    schema_version,
                } => log_database_recovery(
                    database_path,
                    &DatabaseRecovery::IncompatibleVersion {
                        contract_version: *contract_version,
                        schema_version: *schema_version,
                    },
                ),
                ProjectMetaError::Other(_) => {}
            }
            drop(connection);
            return self
                .rebuild_and_finish(
                    root,
                    database_path.to_path_buf(),
                    start,
                    sink,
                    Some(Discard {
                        notice,
                        what: "stale",
                    }),
                )
                .map(Err);
        }
        Ok(Ok(connection))
    }

    /// Shared tail for the missing, corruption and stale-database paths:
    /// with `Some(discard)` it first deletes the bad database (a failure
    /// ends the call as `discarding the <what> project database: ...`), then
    /// rebuilds the database from scratch, stamps the sync time, and emits
    /// the completion signal. `None` is the missing-database path, which
    /// discards nothing. Extracted so the recovery paths cannot drift apart.
    fn rebuild_and_finish(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
        discard: Option<Discard>,
    ) -> Result<SyncedProject, ErrorData> {
        let notice = match discard {
            Some(Discard { notice, what }) => {
                store::discard_corrupt_database(&database_path).map_err(|error| {
                    ErrorData::internal_error(
                        format!("discarding the {what} project database: {error}"),
                        None,
                    )
                })?;
                Some(notice)
            }
            None => None,
        };
        let mut synced = publish_from_scratch(root, database_path, sink).inspect_err(|_| {
            sink.emit(ProgressEvent::Completed {
                phase: "ensuring_current",
            });
        })?;
        synced.discard = notice;
        let duration_ms = crate::clock::saturating_millis(start.elapsed());
        self.stamp_last_sync(duration_ms)?;
        sink.emit(ProgressEvent::Completed {
            phase: "ensuring_current",
        });
        Ok(synced)
    }
}

/// Every state-bearing tool funnels through `ensure_current`, so this is
/// the one place the "no project here" case can be shaped: a path with no
/// activation directory is not an opaque failure of whichever tool was called
/// — it is a project that was never enabled, and the response carries
/// `project_not_enabled` plus the exact remedy (`project_control` with
/// `action: "on"` and the path argument). Any other resolution failure — an
/// unresolvable path, a symlinked activation directory — keeps its plain
/// message and no code, because `project_control` cannot fix it.
pub(crate) fn map_activation_error(error: &project::ActivationError) -> ErrorData {
    if let project::ActivationError::Lock(poisoned) = error {
        return poisoned.clone().into();
    }
    let data = if matches!(error, project::ActivationError::NotActive) {
        error_codes::payload(ErrorCode::ProjectNotEnabled)
    } else {
        None
    };
    ErrorData::invalid_params(error.to_string(), data)
}

/// The tool error for process limits that were never installed and whose
/// `SLUGAUDIT_*` environment is invalid (a library embedder; the shipped
/// binary validates before serving). Logged once here, with the source error.
pub(crate) fn map_process_limits_error(error: &crate::model::EnvConfigError) -> ErrorData {
    if let crate::model::EnvConfigError::WarningLedger(poisoned) = error {
        tracing::error!(%poisoned, "process limits could not be derived");
        return poisoned.clone().into();
    }
    tracing::error!(%error, "process limits environment is invalid");
    ErrorData::internal_error(
        format!("the SLUGAUDIT_* environment is invalid: {error}; fix it and restart the server"),
        None,
    )
}
