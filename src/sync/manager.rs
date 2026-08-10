//! Incremental source synchronization manager for SlugAudit.
//!
//! `SourceSyncManager` owns a `WatchManager` and uses it to avoid full
//! publishes when the filesystem hasn't changed meaningfully. On each
//! `ensure_current` call it inspects the watcher health and the unreconciled
//! event set, then either does a full publish (untrusted watcher) or an
//! incremental reconcile (trusted watcher with pending events).

use super::publish;
use super::revision;
use crate::store;
use crate::watch::{WatchManager, WatchState, WatcherHealth};
use crate::{parse, project};
use rmcp::ErrorData;
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A project brought fully up to date, ready for a tool to query. Defined
/// here (rather than in `tools::context`) so that `SourceSyncManager` can
/// return it without creating a circular module dependency. `tools::context`
/// re-exports it for backward compatibility.
pub struct SyncedProject {
    pub database_path: PathBuf,
    pub revision_id: String,
    #[allow(dead_code)]
    pub root: PathBuf,
}

/// Errors produced by incremental reconciliation.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("revision error: {0}")]
    Revision(#[from] revision::RevisionError),
    #[error("reconcile error: {0}")]
    Reconcile(#[from] super::reconcile::ReconcileError),
    #[error("IO error reading {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Owns a `WatchManager` and provides watcher-aware sync entry points.
/// Cloneable — clones share the underlying watcher state.
#[derive(Clone, Default)]
pub struct SourceSyncManager {
    watch_manager: WatchManager,
}

impl SourceSyncManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new `SourceSyncManager` with a `notify` watcher if the
    /// platform supports it. If the watcher can't be created, the manager
    /// operates in `Unavailable` mode — every `ensure_current` call does a
    /// full publish.
    pub fn with_watcher() -> Self {
        Self {
            watch_manager: WatchManager::with_watcher(),
        }
    }

    /// Start watching `root`. Returns the `WatchState` for the project.
    pub fn activate(&self, root: &Path) -> WatchState {
        self.watch_manager.watch(root)
    }

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
    pub fn ensure_current(&self, path: &str) -> Result<SyncedProject, ErrorData> {
        let root = project::find_project_root(Path::new(path))
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let database_path = project::database_path(&root);

        let mut connection = match store::open_read_write(&database_path) {
            Ok(connection) => connection,
            Err(error) if error.is_corruption() => {
                store::discard_corrupt_database(&database_path).map_err(|error| {
                    ErrorData::internal_error(
                        format!("discarding the corrupt project database: {error}"),
                        None,
                    )
                })?;
                return self.publish_from_scratch(&root, database_path);
            }
            Err(error) => {
                return Err(ErrorData::internal_error(
                    format!("opening the project database for sync: {error}"),
                    None,
                ));
            }
        };

        ensure_project_row(&mut connection, root.as_path())?;

        let state = self.watch_manager.watch(root.as_path());
        let health = state.health();

        let revision_id = match health {
            WatcherHealth::NeedsVerification | WatcherHealth::Desynced => {
                let report = publish::publish(&mut connection, root.as_path(), parse::PACK_VERSION)
                    .map_err(|error| {
                        ErrorData::internal_error(
                            format!("publishing a new revision: {error}"),
                            None,
                        )
                    })?;
                state.set_health(WatcherHealth::Healthy);
                report.revision_id
            }
            WatcherHealth::Healthy => {
                if state.has_unreconciled_events() {
                    self.reconcile(root.as_path(), &state, &mut connection)
                        .map_err(|error| {
                            ErrorData::internal_error(
                                format!("reconciling watcher events: {error}"),
                                None,
                            )
                        })?;
                }
                current_revision_id(&connection)
                    .map_err(|error| {
                        ErrorData::internal_error(
                            format!("reading the current revision: {error}"),
                            None,
                        )
                    })?
                    .ok_or_else(|| {
                        ErrorData::internal_error(
                            "no current revision found after sync — this is unexpected; \
                         try disabling and re-enabling the project",
                            None,
                        )
                    })?
            }
            WatcherHealth::Unavailable => {
                let report = publish::publish(&mut connection, root.as_path(), parse::PACK_VERSION)
                    .map_err(|error| {
                        ErrorData::internal_error(
                            format!("publishing a new revision: {error}"),
                            None,
                        )
                    })?;
                report.revision_id
            }
        };

        drop(connection);

        Ok(SyncedProject {
            database_path,
            revision_id,
            root: root.as_path().to_path_buf(),
        })
    }

    /// Reconciles unreconciled watcher events against the database in a
    /// single atomic revision. Takes the dirty/deleted sets from `state`,
    /// re-hashes dirty files, and only re-indexes those whose content hash
    /// actually changed. Deleted paths are removed (cascading evidence and
    /// dependency edges via foreign keys). If no dirty path's hash differs
    /// from what's stored and there are no deletions, no revision is
    /// published.
    ///
    /// # Errors
    ///
    /// Returns an error if reading a dirty file, querying the database, or
    /// committing the revision fails.
    pub fn reconcile(
        &self,
        root: &Path,
        state: &WatchState,
        connection: &mut Connection,
    ) -> Result<(), SyncError> {
        let (_seq, dirty, deleted) = state.take_dirty();

        if dirty.is_empty() && deleted.is_empty() {
            return Ok(());
        }

        let expected_current = current_revision_id(connection)?;

        let _report = super::reconcile::reconcile_dirty_paths(
            connection,
            root,
            dirty,
            deleted,
            expected_current.as_deref(),
        )?;

        Ok(())
    }

    fn publish_from_scratch(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
    ) -> Result<SyncedProject, ErrorData> {
        let mut connection = store::open_read_write(&database_path).map_err(|error| {
            ErrorData::internal_error(format!("recreating the project database: {error}"), None)
        })?;
        ensure_project_row(&mut connection, root.as_path())?;
        let report = publish::publish(&mut connection, root.as_path(), parse::PACK_VERSION)
            .map_err(|error| {
                ErrorData::internal_error(
                    format!("publishing a replacement revision: {error}"),
                    None,
                )
            })?;
        drop(connection);
        Ok(SyncedProject {
            database_path,
            revision_id: report.revision_id,
            root: root.as_path().to_path_buf(),
        })
    }
}

/// Returns the `revision_id` of the current revision, or `None` if no
/// revision has been published yet.
fn current_revision_id(connection: &Connection) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .optional()
}

/// Ensures the (sole) project row exists on first sync, and verifies the
/// stored `root_path` still matches the canonical root this process
/// resolved on later syncs. Mirrors the logic in `tools::context`.
fn ensure_project_row(connection: &mut Connection, root: &Path) -> Result<(), ErrorData> {
    const CONTRACT_VERSION: i64 = 1;
    const SCHEMA_VERSION: i64 = 1;

    let root_path = root.to_string_lossy();
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });

    connection
        .execute(
            "INSERT OR IGNORE INTO project (\
                id, project_id, root_path, contract_version, schema_version, created_at_unix\
             ) VALUES (1, 'default', ?1, ?2, ?3, ?4)",
            rusqlite::params![
                root_path.as_ref(),
                CONTRACT_VERSION,
                SCHEMA_VERSION,
                created_at
            ],
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("recording project metadata: {error}"), None)
        })?;

    let (stored_root_path, contract_version, schema_version): (String, i64, i64) = connection
        .query_row(
            "SELECT root_path, contract_version, schema_version FROM project WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("reading project metadata: {error}"), None)
        })?;

    if contract_version != CONTRACT_VERSION || schema_version != SCHEMA_VERSION {
        return Err(ErrorData::internal_error(
            format!("unsupported contract/schema version ({contract_version}/{schema_version})"),
            None,
        ));
    }
    if stored_root_path != root_path {
        return Err(ErrorData::invalid_params(
            format!(
                "database at this location belongs to a different project root \
                 (expected {root_path}, found {stored_root_path})"
            ),
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
