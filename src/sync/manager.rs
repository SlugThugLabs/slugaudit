//! Incremental source synchronization manager for SlugAudit.
//!
//! `SourceSyncManager` owns a `WatchManager` and uses it to avoid full
//! publishes when the filesystem hasn't changed meaningfully. The manager
//! state, public accessors, and related types live here. The synchronization
//! workflow is split between the private `orchestration` and `health` child
//! modules, which can access this manager's private state without widening
//! the API or splitting the watcher state machine across unrelated modules.

#[path = "manager_health.rs"]
mod health;
#[path = "manager_orchestration.rs"]
mod orchestration;

use super::reconcile;
use crate::watch::{WatchManager, WatchState};
pub(crate) use orchestration::map_activation_error;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use uuid::Uuid;

/// A project brought fully up to date, ready for a tool to query. Defined
/// here (rather than in `tools::context`) so that `SourceSyncManager` can
/// return it without creating a circular module dependency. `tools::context`
/// re-exports it for backward compatibility.
pub struct SyncedProject {
    /// Path of the SQLite database holding the synced project.
    pub database_path: PathBuf,
    /// The revision the database is current at.
    pub revision_id: String,
}

/// Errors produced by incremental reconciliation.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("reconcile error: {0}")]
    Reconcile(#[from] reconcile::ReconcileError),
    /// The watcher-driven reconcile needs the stat sweep's on-disk path
    /// set for edge resolution; a sweep that cannot complete leaves that
    /// set unknown, so the reconcile fails closed instead of publishing
    /// edges resolved against a set it knows is incomplete.
    #[error("stat sweep error: {0}")]
    Sweep(String),
    /// Same failure as [`SyncError::Sweep`], kept typed: the sweep ran out
    /// of wall-clock budget. A spent budget is a permanent, actionable
    /// condition (`sync_budget_exhausted` + the knob that raises it), while
    /// a stringified sweep failure is not, so the two cannot share a
    /// variant.
    #[error("stat sweep exceeded its wall-clock time budget after {elapsed_ms} ms")]
    SweepBudgetExceeded { elapsed_ms: u128 },
}

impl SyncError {
    /// The elapsed budget when this failure is the budget being exhausted,
    /// else `None`. The two `ErrorData` map sites in `manager_health` pair
    /// this with `crate::error_codes::budget_payload`; the reconcile
    /// pipeline's own budget variant arrives wrapped, so this recurses one
    /// level rather than matching only the top variant.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            SyncError::SweepBudgetExceeded { elapsed_ms } => Some(*elapsed_ms),
            SyncError::Reconcile(error) => error.budget_elapsed_ms(),
            _ => None,
        }
    }
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
    /// Duration of the most recent successful sync, in milliseconds.
    last_sync_duration_ms: std::sync::Arc<AtomicU64>,
    /// How many consecutive full publishes (untrusted-watcher or
    /// Unavailable-path verifications) have run since the last successful
    /// *incremental* reconcile. Every full publish increments it; a
    /// successful incremental reconcile resets it to zero. Exposed through
    /// the `health` MCP tool: a value that climbs on a repo where the
    /// watcher keeps dropping into `Desynced` (e.g. Linux inotify
    /// `fs.inotify.max_user_watches` overflow) is the earliest signal that
    /// every tool call is paying a full-publish cost — see the
    /// full-publish runbook in `docs/TROUBLESHOOTING.md`.
    consecutive_full_publishes: std::sync::Arc<AtomicU64>,
    /// Project databases whose prior-session findings purge has already
    /// run in this process, mapped to the session UUID that ran it. This
    /// is the "purge done" guard for the session-start transition: the
    /// purge must not repeat on every tool call, or one live MCP session
    /// would delete another live session's findings mid-session. Keyed by
    /// database path because one manager can serve several projects, and
    /// valued by session so a (test-only) session change re-arms it.
    /// Clones share the state.
    session_findings_purged: Arc<Mutex<HashMap<PathBuf, Uuid>>>,
    /// The mechanics the auditor registered for this server's session (the
    /// `mechanic` tool), and whether the one-time `report` mechanics note
    /// was delivered. Held here because the manager is the per-server state
    /// every tool already receives; clones share it, and it dies with the
    /// process.
    session_mechanics: crate::project::SessionMechanics,
}

impl SourceSyncManager {
    /// Creates a manager with **no** filesystem watcher running — the default
    /// for tests; the MCP server uses `with_watcher`.
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
            last_sync_duration_ms: std::sync::Arc::new(AtomicU64::new(0)),
            consecutive_full_publishes: std::sync::Arc::new(AtomicU64::new(0)),
            session_findings_purged: Arc::new(Mutex::new(HashMap::new())),
            session_mechanics: crate::project::SessionMechanics::default(),
        }
    }

    /// The mechanics registered for this server's session; see the field.
    pub(crate) fn session_mechanics(&self) -> &crate::project::SessionMechanics {
        &self.session_mechanics
    }

    /// Returns the unix-epoch seconds of the most recent successful
    /// `ensure_current`. Zero before the first sync.
    pub fn last_sync_unix_seconds(&self) -> i64 {
        self.last_sync_unix_seconds.load(Ordering::Relaxed)
    }

    /// Returns the duration in milliseconds of the most recent successful
    /// `ensure_current`. Zero before the first sync.
    pub fn last_sync_duration_ms(&self) -> u64 {
        self.last_sync_duration_ms.load(Ordering::Relaxed)
    }

    /// Number of consecutive full publishes since the last successful
    /// incremental reconcile. Exposed through the `health` MCP tool; a
    /// non-zero value that persists across calls means the watcher is not
    /// being trusted (or is unavailable) and every tool call full-verifies.
    pub fn consecutive_full_publishes(&self) -> u64 {
        self.consecutive_full_publishes.load(Ordering::Relaxed)
    }

    /// Records a full publish (untrusted watcher or Unavailable path).
    /// Called after the publish's database write succeeds, so the counter
    /// only reflects publishes that actually landed.
    fn record_full_publish(&self) {
        self.consecutive_full_publishes
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Resets the consecutive-full-publish counter after a successful
    /// incremental reconcile proves the watcher is trusted and events are
    /// being drained incrementally.
    fn record_incremental_reconcile(&self) {
        self.consecutive_full_publishes.store(0, Ordering::Relaxed);
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

    /// Stop watching `root` and unregisters its state.
    pub fn unwatch(&self, root: &Path) {
        self.watch_manager.unwatch(root);
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
        let canonical = crate::util::canonical_or_self(root);
        self.watch_manager.get(&canonical)
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
#[path = "manager_observability_tests.rs"]
mod observability_tests;

#[cfg(test)]
#[path = "manager_log_tests.rs"]
mod log_tests;
