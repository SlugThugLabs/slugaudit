// slugaudit-line-exception: approved-by=split-judge; reason=per-server sync orchestration, singleflight locking, corrupt-database discard/rebuild recovery, and MCP error mapping
//! Source synchronization manager for SlugAudit: owns per-server state,
//! orchestrates `ensure_current` and `project_control` `on`/`off`, and
//! automatically discards and rebuilds corrupt or stale project databases.

use super::project_meta::{
    ProjectMetaError, ensure_project_row, instance_id_after_sync, revision_after_sync,
};
use super::publish::{PublishError, PublishReport, publish};
use super::publish_log;
use super::result_stores::ResultStoreSlot;
use crate::error_codes::{self, ErrorCode};
use crate::parse;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::project;
use crate::store::{self, StoreError};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Caller-facing record reported on the single call that discarded and rebuilt
/// a corrupt or stale `project.db`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DatabaseDiscard {
    /// Why the database was discarded.
    pub cause: String,
}

/// A project brought fully up to date, ready for a tool to query.
pub struct SyncedProject {
    /// Path of the SQLite database holding the synced project.
    pub database_path: PathBuf,
    /// The revision the database is current at.
    pub revision_id: String,
    /// Per-database instance token (`project.project_id`) captured when this
    /// sync completed.
    pub(crate) db_instance_id: Option<String>,
    /// Set only on the call that discarded and rebuilt the project database.
    pub discard: Option<DatabaseDiscard>,
}

impl SyncedProject {
    /// Path of this process's session findings database beside `project.db`.
    ///
    /// # Errors
    ///
    /// Returns `project_not_enabled` when this process has no session for the
    /// project (`off` ended it after the sync).
    pub fn findings_path(&self) -> Result<PathBuf, ErrorData> {
        session_findings_for(&self.database_path)
    }
}

/// Returns the session findings database path for the project whose index is
/// `database_path`.
pub(crate) fn session_findings_for(database_path: &Path) -> Result<PathBuf, ErrorData> {
    let not_active = || map_activation_error(&project::ActivationError::NotActive);
    let Some(dir) = database_path.parent() else {
        return Err(not_active());
    };
    match project::session_findings_path(dir)? {
        Some(findings) => Ok(findings),
        None => Err(not_active()),
    }
}

/// One database recovery or open-failure event logged once at `WARN`.
pub(super) enum DatabaseRecovery<'a> {
    OpenFailed {
        error: &'a StoreError,
    },
    OpenCorrupt {
        error: &'a StoreError,
    },
    RootMismatch {
        operation: &'static str,
        stored_root: &'a str,
    },
    IncompatibleVersion {
        operation: &'static str,
        contract_version: i64,
        schema_version: i64,
    },
    MetadataCorrupt {
        operation: &'static str,
        error: &'a rusqlite::Error,
    },
    PublishCorrupt {
        error: &'a PublishError,
    },
}

pub(super) fn log_database_recovery(database_path: &Path, recovery: &DatabaseRecovery<'_>) {
    match recovery {
        DatabaseRecovery::OpenFailed { error } => tracing::warn!(
            operation = "open_project_database",
            path = %database_path.display(),
            error_kind = store_error_kind(error),
            error = %error,
            "failed to open project database for sync",
        ),
        DatabaseRecovery::OpenCorrupt { error } => tracing::warn!(
            operation = "open_project_database",
            path = %database_path.display(),
            error_kind = store_error_kind(error),
            error = %error,
            "database is corrupt or outdated; discarding and re-publishing from scratch",
        ),
        DatabaseRecovery::RootMismatch {
            operation,
            stored_root,
        } => tracing::warn!(
            operation,
            path = %database_path.display(),
            error_kind = "root_mismatch",
            stored_root = %stored_root,
            "project database is stale; discarding and rebuilding from source",
        ),
        DatabaseRecovery::IncompatibleVersion {
            operation,
            contract_version,
            schema_version,
        } => tracing::warn!(
            operation,
            path = %database_path.display(),
            error_kind = "incompatible_version",
            contract_version,
            schema_version,
            "project database is stale; discarding and rebuilding from source",
        ),
        DatabaseRecovery::MetadataCorrupt { operation, error } => tracing::warn!(
            operation,
            path = %database_path.display(),
            error_kind = "metadata_corrupt",
            error = %error,
            "database is corrupt while verifying metadata; discarding and re-publishing from scratch",
        ),
        DatabaseRecovery::PublishCorrupt { error } => tracing::warn!(
            operation = "publish_project_database",
            path = %database_path.display(),
            error_kind = "publish_corrupt",
            error = %error,
            "database became corrupt during sync; discarding and re-publishing from scratch",
        ),
    }
}

fn store_error_kind(error: &StoreError) -> &'static str {
    match error {
        StoreError::Open(_) => "open",
        StoreError::Configure(_) => "configure",
        StoreError::Busy(_) => "busy",
        StoreError::Migration(_) => "migration",
        StoreError::Missing => "missing",
        StoreError::Inspect(_) => "inspect",
        StoreError::WalUnavailable { .. } => "wal_unavailable",
    }
}

#[derive(Clone, Copy)]
enum SyncMode {
    EnsureCurrent,
    Enable,
}

impl SyncMode {
    fn metadata_operation(self) -> &'static str {
        match self {
            Self::EnsureCurrent => "verify_project_database",
            Self::Enable => "enable_project",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::EnsureCurrent => "sync",
            Self::Enable => "import",
        }
    }
}

struct Discard {
    notice: DatabaseDiscard,
    what: &'static str,
}

struct PhaseCompletionGuard<'a> {
    sink: &'a dyn ProgressSink,
    phase: &'static str,
}

impl<'a> PhaseCompletionGuard<'a> {
    fn start(sink: &'a dyn ProgressSink, phase: &'static str) -> Self {
        sink.emit(ProgressEvent::Started { phase });
        Self { sink, phase }
    }
}

impl Drop for PhaseCompletionGuard<'_> {
    fn drop(&mut self) {
        self.sink
            .emit(ProgressEvent::Completed { phase: self.phase });
    }
}

/// Owns tool-facing per-server state and provides the sync entry points.
/// Cloneable — clones share the underlying state.
#[derive(Clone, Default)]
pub struct SourceSyncManager {
    last_sync_unix_seconds: Arc<AtomicI64>,
    last_sync_duration_ms: Arc<AtomicU64>,
    session_mechanics: project::SessionMechanics,
    result_stores: ResultStoreSlot,
    sync_locks: Arc<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>>,
}

impl SourceSyncManager {
    /// Creates a manager with no external resources open.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the active-project result store slot.
    #[must_use]
    pub fn result_stores(&self) -> &ResultStoreSlot {
        &self.result_stores
    }

    /// Returns the per-project singleflight mutex for `root`, pruning expired
    /// weak entries.
    pub(super) fn project_sync_lock(&self, root: &Path) -> Result<Arc<Mutex<()>>, ErrorData> {
        let mut table = crate::util::lock(&self.sync_locks, "project_sync_locks")?;
        table.retain(|_, weak| weak.strong_count() > 0);
        if let Some(lock) = table.get(root).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        table.insert(root.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }

    #[cfg(test)]
    pub(crate) fn with_result_store_limits(limits: store::ResultStoreLimits) -> Self {
        Self {
            result_stores: ResultStoreSlot::with_limits(limits),
            ..Self::default()
        }
    }

    pub(crate) fn session_mechanics(&self) -> &project::SessionMechanics {
        &self.session_mechanics
    }

    /// Unix-epoch seconds of the most recent successful sync (0 before first sync).
    #[must_use]
    pub fn last_sync_unix_seconds(&self) -> i64 {
        self.last_sync_unix_seconds.load(Ordering::Relaxed)
    }

    /// Duration in milliseconds of the most recent successful sync (0 before first sync).
    #[must_use]
    pub fn last_sync_duration_ms(&self) -> u64 {
        self.last_sync_duration_ms.load(Ordering::Relaxed)
    }

    fn finish_stamp(&self, start: std::time::Instant) -> Result<(), ErrorData> {
        let duration_ms = crate::clock::duration_millis(start.elapsed())?;
        self.last_sync_unix_seconds
            .store(crate::util::now_unix()?, Ordering::Relaxed);
        self.last_sync_duration_ms
            .store(duration_ms, Ordering::Relaxed);
        Ok(())
    }

    // Owner decision (2026-10-31): the database only matters if it is exactly the same as the code base, and hashing every file on every call is simpler than trusting any shortcut.
    /// Ensures the project containing `path` is fully synchronized and returns
    /// a handle to its current revision.
    ///
    /// # Errors
    ///
    /// Returns `ErrorData` if `path` is not inside an enabled project or sync fails.
    pub fn ensure_current(
        &self,
        path: &str,
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        let start = std::time::Instant::now();
        let _phase_guard = PhaseCompletionGuard::start(sink, "ensuring_current");
        let (root, database_path) = project::resolve_project(Path::new(path))
            .map_err(|error| map_activation_error(&error))?;
        let sync_lock = self.project_sync_lock(root.as_path())?;
        let _sync_guard = crate::util::lock(&sync_lock, "project_sync")?;
        project::begin_process_session(&root).map_err(|error| map_activation_error(&error))?;

        let (synced, report) =
            self.run_locked_sync(&root, database_path, SyncMode::EnsureCurrent, start, sink)?;
        tracing::debug!(
            revision_id = %synced.revision_id,
            root = %root.as_path().display(),
            added = report.added,
            modified = report.modified,
            deleted = report.deleted,
            "ensure_current completed",
        );
        Ok(synced)
    }

    /// Runs the initial import for `project_control` `on` under the per-project
    /// sync lock, recovering a corrupt or stale `project.db` if needed.
    ///
    /// # Errors
    ///
    /// Returns `ErrorData` if opening, recovering, or importing the project
    /// database fails.
    pub fn sync_enabled_project(
        &self,
        root: &project::ProjectRoot,
        sink: &dyn ProgressSink,
    ) -> Result<(PublishReport, Option<DatabaseDiscard>), ErrorData> {
        let start = std::time::Instant::now();
        let sync_lock = self.project_sync_lock(root.as_path())?;
        let _sync_guard = crate::util::lock(&sync_lock, "project_sync")?;
        let database_path = project::database_path(root);
        let (synced, report) =
            self.run_locked_sync(root, database_path, SyncMode::Enable, start, sink)?;
        Ok((report, synced.discard))
    }

    /// Releases all in-memory manager resources (session mechanics and open
    /// result stores) for `root` before `project_control` `off` deletes
    /// `.planning/slugaudit`.
    ///
    /// # Errors
    ///
    /// Returns `ErrorData` if an internal mutex is poisoned.
    pub fn release_disabled_project(&self, root: &project::ProjectRoot) -> Result<(), ErrorData> {
        let sync_lock = self.project_sync_lock(root.as_path())?;
        let _sync_guard = crate::util::lock(&sync_lock, "project_sync")?;
        let database_path = project::database_path(root);
        self.session_mechanics.clear_project(root.as_path())?;
        if let Some(runtime_dir) = database_path.parent() {
            self.result_stores.release_dir(runtime_dir)?;
        }
        Ok(())
    }

    fn run_locked_sync(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
        mode: SyncMode,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<(SyncedProject, PublishReport), ErrorData> {
        let open_result = match mode {
            SyncMode::EnsureCurrent => store::open_existing_read_write(&database_path),
            SyncMode::Enable => store::open_read_write(&database_path),
        };
        let mut connection = match open_result {
            Ok(conn) => conn,
            Err(StoreError::Missing) => {
                return self.rebuild_and_finish(root, database_path, start, sink, None);
            }
            Err(error) if error.is_corruption() => {
                log_database_recovery(
                    &database_path,
                    &DatabaseRecovery::OpenCorrupt { error: &error },
                );
                let discard = Discard {
                    notice: DatabaseDiscard {
                        cause: format!("the project database is corrupt or outdated: {error}"),
                    },
                    what: "corrupt or outdated",
                };
                return self.rebuild_and_finish(root, database_path, start, sink, Some(discard));
            }
            Err(error) => {
                log_database_recovery(
                    &database_path,
                    &DatabaseRecovery::OpenFailed { error: &error },
                );
                let prefix = match mode {
                    SyncMode::EnsureCurrent => "opening the project database for sync",
                    SyncMode::Enable => "database",
                };
                return Err(ErrorData::internal_error(
                    format!("{prefix}: {error}"),
                    None,
                ));
            }
        };

        if let Err(meta_error) = ensure_project_row(&mut connection, root.as_path()) {
            let discard =
                classify_metadata_discard(&database_path, mode.metadata_operation(), meta_error)?;
            drop(connection);
            return self.rebuild_and_finish(root, database_path, start, sink, Some(discard));
        }

        match publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink) {
            Ok(report) => {
                let revision_id = revision_after_sync(&connection, &database_path)?;
                let db_instance_id = instance_id_after_sync(&connection, &database_path)?;
                self.finish_stamp(start)?;
                Ok((
                    SyncedProject {
                        database_path,
                        revision_id,
                        db_instance_id,
                        discard: None,
                    },
                    report,
                ))
            }
            Err(error) if error.is_corruption() => {
                log_database_recovery(
                    &database_path,
                    &DatabaseRecovery::PublishCorrupt { error: &error },
                );
                drop(connection);
                let discard = Discard {
                    notice: DatabaseDiscard {
                        cause: format!(
                            "the project database became corrupt during {}: {error}",
                            mode.noun()
                        ),
                    },
                    what: "corrupt",
                };
                self.rebuild_and_finish(root, database_path, start, sink, Some(discard))
            }
            Err(error) => Err(match mode {
                SyncMode::EnsureCurrent => map_publish_error(root.as_path(), &error, "sync"),
                SyncMode::Enable => map_import_publish_error(root.as_path(), &error),
            }),
        }
    }

    #[cfg(test)]
    pub(super) fn verify_metadata_or_rebuild(
        &self,
        root: &project::ProjectRoot,
        database_path: &Path,
        mut connection: rusqlite::Connection,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        if let Err(meta_error) = ensure_project_row(&mut connection, root.as_path()) {
            let discard =
                classify_metadata_discard(database_path, "verify_project_database", meta_error)?;
            drop(connection);
            return self
                .rebuild_and_finish(
                    root,
                    database_path.to_path_buf(),
                    start,
                    sink,
                    Some(discard),
                )
                .map(|(synced, _)| synced);
        }
        let revision_id = revision_after_sync(&connection, database_path)?;
        let db_instance_id = instance_id_after_sync(&connection, database_path)?;
        Ok(SyncedProject {
            database_path: database_path.to_path_buf(),
            revision_id,
            db_instance_id,
            discard: None,
        })
    }

    fn rebuild_and_finish(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
        start: std::time::Instant,
        sink: &dyn ProgressSink,
        discard: Option<Discard>,
    ) -> Result<(SyncedProject, PublishReport), ErrorData> {
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
        let (mut synced, report) = publish_from_scratch_with_report(root, database_path, sink)?;
        self.finish_stamp(start)?;
        synced.discard = notice;
        Ok((synced, report))
    }
}

fn classify_metadata_discard(
    database_path: &Path,
    operation: &'static str,
    metadata_error: ProjectMetaError,
) -> Result<Discard, ErrorData> {
    let (cause, what) = match metadata_error {
        ProjectMetaError::RootMismatch { stored_root } => {
            log_database_recovery(
                database_path,
                &DatabaseRecovery::RootMismatch {
                    operation,
                    stored_root: &stored_root,
                },
            );
            (
                "the project database was built for a different project root".to_owned(),
                "stale",
            )
        }
        ProjectMetaError::IncompatibleVersion {
            contract_version,
            schema_version,
        } => {
            log_database_recovery(
                database_path,
                &DatabaseRecovery::IncompatibleVersion {
                    operation,
                    contract_version,
                    schema_version,
                },
            );
            (
                format!(
                    "the project database has an incompatible version \
                     (contract_version {contract_version}, schema_version {schema_version})"
                ),
                "stale",
            )
        }
        ProjectMetaError::Corrupt(error) => {
            log_database_recovery(
                database_path,
                &DatabaseRecovery::MetadataCorrupt {
                    operation,
                    error: &error,
                },
            );
            (
                format!("the project database is corrupt: {error}"),
                "corrupt",
            )
        }
        ProjectMetaError::Other(err) => return Err(err),
    };
    Ok(Discard {
        notice: DatabaseDiscard { cause },
        what,
    })
}

fn publish_error_data(message: String, error: &PublishError) -> ErrorData {
    let budget = error.budget_elapsed_ms();
    let payload = if budget.is_some() {
        error_codes::budget_payload(budget)
    } else if matches!(
        error,
        PublishError::ChangedDuringSample { .. }
            | PublishError::Revision(super::revision::RevisionError::StaleBaseline { .. })
    ) {
        Some(error_codes::payload(ErrorCode::StaleRevision))
    } else {
        None
    };
    ErrorData::internal_error(message, payload)
}

/// Maps a `publish()` error on a sync or rebuild path into an `ErrorData`
/// carrying the structured error code and full [`publish_log::failure_context`].
pub(crate) fn map_publish_error(root: &Path, error: &PublishError, operation: &str) -> ErrorData {
    publish_error_data(
        format!(
            "publishing a new revision: {error} ({operation}; {})",
            publish_log::failure_context(root, error)
        ),
        error,
    )
}

fn map_import_publish_error(root: &Path, error: &PublishError) -> ErrorData {
    publish_error_data(
        format!(
            "import: {error} ({})",
            publish_log::failure_context(root, error)
        ),
        error,
    )
}

fn publish_from_scratch_with_report(
    root: &project::ProjectRoot,
    database_path: PathBuf,
    sink: &dyn ProgressSink,
) -> Result<(SyncedProject, PublishReport), ErrorData> {
    let mut connection = store::open_read_write(&database_path).map_err(|error| {
        if matches!(error, StoreError::Open(_))
            && database_path.parent().is_some_and(|dir| !dir.is_dir())
        {
            return ErrorData::invalid_params(
                "the project was turned off during this call: .planning/slugaudit no longer \
                 exists. Call project_control with action \"on\" to enable it again",
                Some(error_codes::payload(ErrorCode::ProjectNotEnabled)),
            );
        }
        ErrorData::internal_error(format!("recreating the project database: {error}"), None)
    })?;
    ensure_project_row(&mut connection, root.as_path())?;
    let report =
        publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink).map_err(|error| {
            map_publish_error(root.as_path(), &error, "republishing after corruption")
        })?;
    let db_instance_id = instance_id_after_sync(&connection, &database_path)?;
    drop(connection);
    let synced = SyncedProject {
        database_path,
        revision_id: report.revision_id.clone(),
        db_instance_id,
        discard: None,
    };
    Ok((synced, report))
}

/// Rebuilds a project's database from scratch after the previous one was
/// missing or discarded (test helper).
#[cfg(test)]
pub(crate) fn publish_from_scratch(
    root: &project::ProjectRoot,
    database_path: PathBuf,
    sink: &dyn ProgressSink,
) -> Result<SyncedProject, ErrorData> {
    publish_from_scratch_with_report(root, database_path, sink).map(|(synced, _)| synced)
}

/// Shapes `ActivationError` into `ErrorData` with `project_not_enabled` when
/// the project has not been enabled.
pub(crate) fn map_activation_error(error: &project::ActivationError) -> ErrorData {
    if let project::ActivationError::Lock(poisoned) = error {
        return poisoned.clone().into();
    }
    let data = if matches!(error, project::ActivationError::NotActive) {
        Some(error_codes::payload(ErrorCode::ProjectNotEnabled))
    } else {
        None
    };
    ErrorData::invalid_params(error.to_string(), data)
}

/// Shapes an invalid `SLUGAUDIT_*` environment override into an `ErrorData`.
pub(crate) fn map_process_limits_error(error: &crate::model::EnvConfigError) -> ErrorData {
    if let crate::model::EnvConfigError::WarningLedger(poisoned) = error {
        return poisoned.clone().into();
    }
    ErrorData::internal_error(
        format!("the SLUGAUDIT_* environment is invalid: {error}; fix it and restart the server"),
        None,
    )
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "manager_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[path = "manager_log_tests.rs"]
mod log_tests;

#[cfg(test)]
#[path = "discard_notice_tests.rs"]
mod discard_notice_tests;
