//! Source synchronization manager for SlugAudit.
//!
//! `SourceSyncManager` owns the per-server state every tool call shares
//! (the last-sync stamps, session mechanics, the mechanic scan cache, and
//! the paged-result store) and exposes the one sync entry point, `ensure_current`. Every call hashes
//! the project and publishes the delta; there is no filesystem watcher and
//! no timestamp shortcut, so an answer is proven against the source before
//! it is served.

#[path = "manager_orchestration.rs"]
mod orchestration;

use super::discard_notice::DatabaseDiscard;
pub(crate) use orchestration::{map_activation_error, map_process_limits_error};
use rmcp::ErrorData;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// A project brought fully up to date, ready for a tool to query. Defined
/// here (rather than in `tools::context`) so that `SourceSyncManager` can
/// return it without creating a circular module dependency. `tools::context`
/// re-exports it for backward compatibility.
pub struct SyncedProject {
    /// Path of the SQLite database holding the synced project.
    pub database_path: PathBuf,
    /// The revision the database is current at.
    pub revision_id: String,
    /// Set only on the call that discarded and rebuilt the project database
    /// (corruption, version bump, root mismatch); `None` on every other call.
    pub discard: Option<DatabaseDiscard>,
}

impl SyncedProject {
    /// Path of this process's session findings database, beside the project
    /// database. It need not exist: no finding was written yet in this
    /// session.
    ///
    /// # Errors
    ///
    /// Returns `project_not_enabled` when this process has no session for the
    /// project: `off` ended it after the sync.
    pub fn findings_path(&self) -> Result<PathBuf, ErrorData> {
        session_findings_for(&self.database_path)
    }
}

/// The session findings database of the project whose `project.db` is
/// `database_path`.
///
/// # Errors
///
/// Returns `project_not_enabled` when this process has no session for the
/// project.
pub(crate) fn session_findings_for(database_path: &Path) -> Result<PathBuf, ErrorData> {
    database_path
        .parent()
        .and_then(crate::project::session_findings_path)
        .ok_or_else(|| map_activation_error(crate::project::ActivationError::NotActive))
}

/// Owns the tool-facing per-server state and provides the sync entry point.
/// Cloneable — clones share the underlying state.
#[derive(Clone, Default)]
pub struct SourceSyncManager {
    /// Unix-epoch seconds of the most recent successful `ensure_current`,
    /// regardless of project. Exposed through the `health` MCP tool so
    /// operators can detect "the server has been up but hasn't actually
    /// synced anything for N seconds" without parsing MCP logs.
    ///
    /// Stamped after the database write succeeds, not before, so the
    /// timestamp is consistent with what the database sees as the most
    /// recent revision.
    last_sync_unix_seconds: Arc<AtomicI64>,
    /// Duration of the most recent successful sync, in milliseconds.
    last_sync_duration_ms: Arc<AtomicU64>,
    /// The mechanics the auditor registered for this server's session (the
    /// `mechanic` tool). Held here because the manager is the per-server
    /// state every tool already receives; clones share it, and it dies with
    /// the process.
    session_mechanics: crate::project::SessionMechanics,
    /// Complete scan results of the registered mechanics, per revision; see
    /// `project::MechanicScanCache`. Held beside the session mechanics for
    /// the same reason: per-server state every tool already receives.
    mechanic_scan_cache: crate::project::MechanicScanCache,
    /// The lazily opened result store for paged results; see
    /// `result_stores`. Per-server state like the fields above.
    result_stores: super::result_stores::ResultStoreSlot,
}

impl SourceSyncManager {
    /// Creates a manager with no external resources open.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The result store slot; see the field.
    #[must_use]
    pub fn result_stores(&self) -> &super::result_stores::ResultStoreSlot {
        &self.result_stores
    }

    /// A manager whose result store obeys `limits`.
    #[cfg(test)]
    pub(crate) fn with_result_store_limits(limits: crate::store::ResultStoreLimits) -> Self {
        Self {
            result_stores: super::result_stores::ResultStoreSlot::with_limits(limits),
            ..Self::default()
        }
    }

    /// The mechanics registered for this server's session; see the field.
    pub(crate) fn session_mechanics(&self) -> &crate::project::SessionMechanics {
        &self.session_mechanics
    }

    /// The registered mechanics' scan cache; see the field.
    pub(crate) fn mechanic_scan_cache(&self) -> &crate::project::MechanicScanCache {
        &self.mechanic_scan_cache
    }

    /// A manager whose mechanic scan cache holds at most `budget` bytes.
    #[cfg(test)]
    pub(crate) fn with_mechanic_cache_budget(budget: usize) -> Self {
        Self {
            mechanic_scan_cache: crate::project::MechanicScanCache::with_budget(budget),
            ..Self::default()
        }
    }

    /// Returns the unix-epoch seconds of the most recent successful
    /// `ensure_current`. Zero before the first sync.
    #[must_use]
    pub fn last_sync_unix_seconds(&self) -> i64 {
        self.last_sync_unix_seconds.load(Ordering::Relaxed)
    }

    /// Returns the duration in milliseconds of the most recent successful
    /// `ensure_current`. Zero before the first sync.
    #[must_use]
    pub fn last_sync_duration_ms(&self) -> u64 {
        self.last_sync_duration_ms.load(Ordering::Relaxed)
    }

    fn stamp_last_sync(&self, duration_ms: u64) {
        self.last_sync_unix_seconds
            .store(crate::util::now_unix(), Ordering::Relaxed);
        self.last_sync_duration_ms
            .store(duration_ms, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::progress::NoopProgressSink;

    /// A valid database whose metadata names a different project root is
    /// stale, and the recovery path discards it. When the database path is a
    /// directory the discard cannot unlink it, so the failure must reach the
    /// caller with the underlying discard error intact.
    #[test]
    fn a_failed_stale_database_discard_reports_the_exact_error() {
        let project = crate::sync::test_support::create_project();
        let root = crate::project::ProjectRoot::resolve(project.path()).expect("project root");
        let database_path = crate::project::database_path(&root);
        let connection = crate::store::open_read_write(&database_path).expect("open database");
        connection
            .execute(
                "INSERT INTO project VALUES (1, 'default', '/elsewhere', 1, 1, 0)",
                [],
            )
            .expect("stale metadata row");
        std::fs::remove_file(&database_path).expect("unlink the database file");
        std::fs::create_dir(&database_path).expect("directory at the database path");
        let manager = SourceSyncManager::default();

        let Err(error) = manager.verify_metadata_or_rebuild(
            &root,
            &database_path,
            connection,
            std::time::Instant::now(),
            &NoopProgressSink,
        ) else {
            panic!("the directory at the database path cannot be discarded");
        };

        let discard_error = std::fs::remove_file(&database_path)
            .expect_err("the directory at the database path must not unlink");
        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            error.message,
            format!("discarding the stale project database: {discard_error}")
        );
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "manager_log_tests.rs"]
mod log_tests;
