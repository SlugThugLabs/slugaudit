//! Incremental source synchronization manager for SlugAudit.
//!
//! `SourceSyncManager` owns a `WatchManager` and uses it to avoid full
//! publishes when the filesystem hasn't changed meaningfully. On each
//! `ensure_current` call it inspects the watcher health and the unreconciled
//! event set, then either does a full publish (untrusted watcher) or an
//! incremental reconcile (trusted watcher with pending events).
// slugaudit-line-exception: approved-by=agent; reason=ensure_current's three-branch match is the sync orchestrator's hot path; trace sites and stamp_last_sync belong next to the code paths they cover

use super::manager_meta::{current_revision_id, ensure_project_row};
use super::publish;
use super::revision;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::store;
use crate::watch::{WatchManager, WatchState, WatcherHealth};
use crate::{parse, project};
use rmcp::ErrorData;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
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
    /// Unix-epoch seconds of the most recent successful `ensure_current`,
    /// regardless of project. Exposed through the `health` MCP tool so
    /// operators can detect "the server has been up but hasn't actually
    /// synced anything for N seconds" without parsing MCP logs.
    ///
    /// Stamped after the database write succeeds, not before, so the
    /// timestamp is consistent with what the database sees as the most
    /// recent revision.
    last_sync_unix_seconds: std::sync::Arc<AtomicI64>,
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
            last_sync_unix_seconds: std::sync::Arc::new(AtomicI64::new(0)),
        }
    }

    /// Returns the unix-epoch seconds of the most recent successful
    /// `ensure_current`. Zero before the first sync.
    pub fn last_sync_unix_seconds(&self) -> i64 {
        self.last_sync_unix_seconds.load(Ordering::Relaxed)
    }

    /// Returns the most recently synced project's `WatchState`, or `None`
    /// if no project has been synced yet. The "active project" is the
    /// one `ensure_current` last succeeded for; in the current
    /// single-active-project model, returns the single registered state.
    pub fn active_watch_state(&self) -> Option<WatchState> {
        self.watch_manager
            .iter()
            .into_iter()
            .next()
            .map(|(_, state)| state)
    }

    /// Used by `health` to enumerate every watched project's state for
    /// observability. Iterating owns the inner lock through
    /// `lock_or_recover`, so a panic inside the iterator's `for` body is
    /// recovered on the next iteration just like any other caller.
    pub fn watch_states_snapshot(&self) -> Vec<crate::watch::ProjectWatchState> {
        self.watch_manager.snapshot_all()
    }

    /// Start watching `root`. Returns the `WatchState` for the project.
    pub fn activate(&self, root: &Path) -> WatchState {
        self.watch_manager.watch(root)
    }

    /// Returns the `WatchState` for a previously-watched project root,
    /// or `None` if the project has not been watched yet.
    ///
    /// Unlike `activate`, this does not register a new watch and does
    /// not set the watcher's health to `NeedsVerification`. Callers
    /// that want to surface state on a known active project (e.g. the
    /// `health` MCP tool) should use this; callers that want to ensure
    /// the project is being watched on this connection should use
    /// `activate`.
    pub fn watch_state_for(&self, root: &Path) -> Option<WatchState> {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        self.watch_manager.get(&canonical)
    }

    fn stamp_last_sync(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
            });
        self.last_sync_unix_seconds.store(now, Ordering::Relaxed);
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
    pub fn ensure_current(
        &self,
        path: &str,
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        sink.emit(ProgressEvent::Started {
            phase: "ensuring_current",
        });
        let root = project::find_project_root(Path::new(path))
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let database_path = project::database_path(&root);

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
                let synced = self.publish_from_scratch(&root, database_path, sink)?;
                self.stamp_last_sync();
                sink.emit(ProgressEvent::Completed {
                    phase: "ensuring_current",
                });
                return Ok(synced);
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

        ensure_project_row(&mut connection, root.as_path()).inspect_err(|error| {
            tracing::warn!(
                database_path = %database_path.display(),
                error = %error.message,
                "failed to record project metadata",
            );
        })?;

        let state = self.watch_manager.watch(root.as_path());
        let health = state.health();

        let revision_id = match health {
            WatcherHealth::NeedsVerification | WatcherHealth::Desynced => {
                tracing::info!(
                    ?health,
                    root = %root.as_path().display(),
                    "watcher untrusted; running full verification",
                );
                let report =
                    publish::publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink)
                        .map_err(|error| {
                            tracing::warn!(
                                root = %root.as_path().display(),
                                error = %error,
                                "full publish failed",
                            );
                            ErrorData::internal_error(
                                format!("publishing a new revision: {error}"),
                                None,
                            )
                        })?;
                // Drain any events that arrived during the full verification.
                // `publish` walks the filesystem and parses files, which takes
                // time — events can arrive while it runs. If we don't drain
                // them here, they'd wait until the next MCP call to be
                // reconciled, leaving the database stale in the interim.
                self.reconcile(root.as_path(), &state, &mut connection)
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
                report.revision_id
            }
            WatcherHealth::Healthy => {
                if state.has_unreconciled_events()
                    && let Err(error) = self.reconcile(root.as_path(), &state, &mut connection)
                {
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
                current_revision_id(&connection)
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
                    })?
            }
            WatcherHealth::Unavailable => {
                tracing::info!(
                    root = %root.as_path().display(),
                    "watcher unavailable; running full publish",
                );
                let report =
                    publish::publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink)
                        .map_err(|error| {
                            tracing::warn!(
                                root = %root.as_path().display(),
                                error = %error,
                                "publish on Unavailable path failed",
                            );
                            ErrorData::internal_error(
                                format!("publishing a new revision: {error}"),
                                None,
                            )
                        })?;
                report.revision_id
            }
        };

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
            root: root.as_path().to_path_buf(),
        })
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
        let expected_current = current_revision_id(connection)?;

        super::reconcile::sync_with_barrier(state, |dirty, deleted| {
            super::reconcile::reconcile_dirty_paths(
                connection,
                root,
                dirty,
                deleted,
                expected_current.as_deref(),
            )?;
            Ok(())
        })?;

        Ok(())
    }

    fn publish_from_scratch(
        &self,
        root: &project::ProjectRoot,
        database_path: PathBuf,
        sink: &dyn ProgressSink,
    ) -> Result<SyncedProject, ErrorData> {
        let mut connection = store::open_read_write(&database_path).map_err(|error| {
            ErrorData::internal_error(format!("recreating the project database: {error}"), None)
        })?;
        ensure_project_row(&mut connection, root.as_path())?;
        let report = publish::publish(&mut connection, root.as_path(), parse::PACK_VERSION, sink)
            .map_err(|error| {
            ErrorData::internal_error(format!("publishing a replacement revision: {error}"), None)
        })?;
        drop(connection);
        Ok(SyncedProject {
            database_path,
            revision_id: report.revision_id,
            root: root.as_path().to_path_buf(),
        })
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "manager_observability_tests.rs"]
mod observability_tests;
