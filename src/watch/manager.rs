//! Filesystem watcher manager. Owns the `notify` watcher and dispatches
//! events to per-project `WatchState`. Runs on a dedicated background
//! thread; the MCP server interacts with it only through this manager.
//!
//! The manager also owns each project's watch *scope*: the set of
//! directories that actually carry watches. Discovery indexes only the
//! files its walker yields, so the watcher registers watches only on the
//! same set (see `super::scope`), and drops events for paths the project
//! ignores — a gitignored `target/` must not be watched, and must never
//! mark anything dirty.
//!
//! The lock-held machinery lives in child modules: event dispatch and
//! deferral in [`events`], scope registration and refresh in [`scopes`].

mod events;
mod scopes;

use super::errors::log_notify_failure;
use super::scope::{self, WatchScope};
use super::state::WatchState;
use crate::ignore_rules::IgnoreRules;
use crate::util::{lock_or_recover, redacted_path};
#[cfg(test)]
use notify::EventKind;
use notify::{Config, Event, RecommendedWatcher, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Manages filesystem watchers for all active projects. One instance lives
/// on the `SlugAuditServer` (or its sync manager). Each project gets its
/// own `WatchState`; the manager routes `notify` events to the right state.
#[derive(Clone, Default)]
pub struct WatchManager {
    inner: Arc<Mutex<WatchManagerInner>>,
    /// Events and watcher errors that arrived while a tool thread held
    /// `inner`. They wait here (a lock that is never held across anything
    /// that blocks, so the notify thread can always take it) and are
    /// applied, in order, by whichever thread next holds `inner`.
    deferred: Arc<Mutex<Deferred>>,
}

/// The most events held back while the manager lock is busy. Past this the
/// events are counted, not kept, and the loss is made explicit by marking
/// the projects `Desynced`.
const MAX_DEFERRED_EVENTS: usize = 4096;

#[derive(Default)]
struct Deferred {
    events: Vec<Event>,
    /// A watcher error arrived while a tool thread held `inner`. Drained by
    /// the next lock holder, which marks every project `Desynced`.
    desync_requested: bool,
    /// Events that did not fit under `MAX_DEFERRED_EVENTS`.
    overflowed: u64,
}

#[derive(Default)]
pub(super) struct WatchManagerInner {
    /// Per-project watch state and derived scope/rules, keyed by
    /// canonical project root.
    pub(super) projects: HashMap<PathBuf, ProjectWatchInfo>,
    /// The underlying notify watcher. May be absent if the platform
    /// doesn't support filesystem watching.
    pub(super) watcher: Option<RecommendedWatcher>,
}

/// Everything the manager tracks for one active project: the watch state,
/// the current scope/rules, and a flag that says an ignore file changed
/// and the scope must be recomputed on the next tool-thread pass.
#[derive(Default)]
pub(super) struct ProjectWatchInfo {
    pub(super) state: WatchState,
    /// The last computed scope. `None` until the first computation.
    pub(super) scope: Option<WatchScope>,
    /// The rules built from the scope's ignore files.
    pub(super) rules: Option<Arc<IgnoreRules>>,
    /// Set when an event for a `.gitignore`/`.ignore` file was dropped in
    /// — the next `refresh_scope` from a tool thread recomputes scope and
    /// rules before anything else is reconciled.
    pub(super) needs_scope_refresh: bool,
    /// Bumped on every set of `needs_scope_refresh`. A scope walk captures
    /// it before starting and compares it after re-locking: if it moved, an
    /// ignore-file event arrived while the walk was reading the old rules,
    /// so the refresh stays pending rather than being cleared.
    pub(super) scope_generation: u64,
}

impl WatchManager {
    /// Creates a `WatchManager` with **no** `notify` watcher running — the
    /// default for this module's tests; production constructs through
    /// [`Self::with_watcher`] (or `Default`, which `SourceSyncManager`
    /// derives). Not public: a second name for `default()` on the shipped
    /// surface had no caller.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Creates a new WatchManager with a `notify` watcher if the platform
    /// supports it. Returns `Unavailable` health for the first project if
    /// the watcher can't be created.
    pub fn with_watcher() -> Self {
        let inner = WatchManagerInner {
            projects: HashMap::new(),
            watcher: None,
        };
        let manager = Self {
            inner: Arc::new(Mutex::new(inner)),
            ..Self::default()
        };

        // Try to create a notify watcher. If it fails, we'll operate in
        // Unavailable mode — the sync layer will do full verification on
        // every call.
        let manager_clone = manager.clone();
        match RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| match result {
                Ok(event) => manager_clone.handle_event(event),
                // Watcher error (e.g., queue overflow, watch removed): mark
                // every project `Desynced` without ever blocking this event
                // loop on the manager lock — see `handle_notify_error`.
                Err(error) => manager_clone.handle_notify_error(&error),
            },
            Config::default(),
        ) {
            Ok(watcher) => {
                let mut guard = lock_or_recover(&manager.inner, "watch_manager_inner");
                guard.watcher = Some(watcher);
            }
            Err(error) => {
                // The category is carried, not just the consequence: inotify
                // exhaustion, a permissions problem and a missing kernel
                // feature are indistinguishable without it.
                log_notify_failure("create_watcher", "unscoped", &error);
            }
        }

        manager
    }

    /// Start watching `root`. Returns the `WatchState` for the project.
    /// If the watcher is unavailable, returns a state with `Unavailable`
    /// health. Computes the watch scope immediately — under the process-wide
    /// watch limits — and prunes watches on ignored/excluded directories.
    pub fn watch(&self, root: &Path) -> WatchState {
        let limits = crate::model::process_limits();
        self.watch_with_scope_limits(root, limits.max_watch_entries, limits.max_watch_depth)
    }

    /// Stop watching `root` and remove its state.
    pub fn unwatch(&self, root: &Path) {
        let canonical = crate::util::canonical_or_self(root);

        let mut guard = lock_or_recover(&self.inner, "watch_manager_inner");

        // The project is dropped from `projects` either way, so a failed
        // unwatch leaves the watcher delivering events for a project this
        // manager no longer tracks.
        if let Some(ref mut watcher) = guard.watcher
            && let Err(error) = watcher.unwatch(&canonical)
        {
            log_notify_failure("unwatch_project", &redacted_path(&canonical), &error);
        }

        guard.projects.remove(&canonical);
    }

    /// Get the `WatchState` for `root`, if it exists.
    ///
    /// Drains queued events/errors first: `health` reads through here and
    /// must not report a watcher `Healthy` after an error that was queued
    /// while a tool thread held the lock.
    pub fn get(&self, root: &Path) -> Option<WatchState> {
        let canonical = crate::util::canonical_or_self(root);
        let mut guard = lock_or_recover(&self.inner, "watch_manager_inner");
        self.apply_deferred(&mut guard);
        guard
            .projects
            .get(&canonical)
            .map(|info| info.state.clone())
    }

    /// Iterates every active `WatchState`. Used by the `health` MCP
    /// tool to enumerate the projects currently registered with the
    /// server. Order is unspecified — callers must not depend on it.
    ///
    /// Returns owned `WatchState` clones (the manager itself stays
    /// alive). Each clone is independent of the manager's lifetime;
    /// dropping, cloning, and querying are all safe.
    pub fn iter(&self) -> Vec<(PathBuf, WatchState)> {
        let mut guard = lock_or_recover(&self.inner, "watch_manager_inner");
        self.apply_deferred(&mut guard);
        guard
            .projects
            .iter()
            .map(|(root, info)| (root.clone(), info.state.clone()))
            .collect()
    }

    /// Snapshots the inner data of every active `WatchState` without
    /// holding the lock across the iteration. Used for observability
    /// where the caller wants an immediate cross-project view.
    pub fn snapshot_all(&self) -> Vec<crate::watch::ProjectWatchState> {
        let mut guard = lock_or_recover(&self.inner, "watch_manager_inner");
        self.apply_deferred(&mut guard);
        guard
            .projects
            .values()
            .map(|info| info.state.snapshot())
            .collect()
    }

    /// Recomputes the watch scope and rules for `root` when an ignore
    /// file changed (or when no scope exists yet). Safe to call from a
    /// tool thread — it may unwatch and re-watch directories, and
    /// `notify`'s `unwatch` blocks on the event loop, so it must never
    /// run on the notify callback thread. The sync layer calls this
    /// before reconciling.
    pub fn refresh_scope(&self, root: &Path) {
        let limits = crate::model::process_limits();
        self.refresh_scope_with_limits(root, limits.max_watch_entries, limits.max_watch_depth);
    }

    /// True when `root`'s current watch scope stopped at a documented
    /// entry or depth limit. Directories beyond the limit are unwatched;
    /// the stat sweep still runs on every call, so freshness is not lost.
    pub fn scope_limited(&self, root: &Path) -> bool {
        let canonical = crate::util::canonical_or_self(root);
        scope::scope_limited(
            &lock_or_recover(&self.inner, "watch_manager_inner"),
            &canonical,
        )
    }

    /// The `IgnoreRules` currently in effect for `root`, if the project
    /// is registered. Used by the sync layer to filter dirty paths before
    /// indexing them.
    pub fn rules_for(&self, root: &Path) -> Option<Arc<IgnoreRules>> {
        let canonical = crate::util::canonical_or_self(root);
        let guard = lock_or_recover(&self.inner, "watch_manager_inner");
        guard
            .projects
            .get(&canonical)
            .and_then(|info| info.rules.clone())
    }
}

#[cfg(test)]
#[path = "manager_callback_tests.rs"]
mod callback_tests;

#[cfg(test)]
#[path = "manager_event_tests.rs"]
mod event_tests;
