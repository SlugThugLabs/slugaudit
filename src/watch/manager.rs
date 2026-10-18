// slugaudit-line-exception: approved-by=slugthug; reason=the notify watcher lifecycle, per-project watch states and scope refresh all run under one manager lock, and the rule that unwatch happens only on a tool thread (never inside the notify callback, which deadlocks) is enforced by keeping every lock holder in this file; the pure scope diff and event classification live in event_filter.rs
//!
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

use super::event_filter::{PathClass, classify_path, watch_dir_changes};
use super::path::normalize_relative_path;
use super::scope::WatchScope;
use super::state::WatchState;
use super::types::WatcherHealth;
use crate::ignore_rules::IgnoreRules;
use crate::util::{lock_or_recover, redacted_path};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

/// Manages filesystem watchers for all active projects. One instance lives
/// on the `SlugAuditServer` (or its sync manager). Each project gets its
/// own `WatchState`; the manager routes `notify` events to the right state.
#[derive(Clone, Default)]
pub struct WatchManager {
    inner: Arc<Mutex<WatchManagerInner>>,
    /// Events that arrived while a tool thread held `inner`. They wait here
    /// (a lock that is never held across anything that blocks, so the notify
    /// thread can always take it) and are applied, in order, by whichever
    /// thread next holds `inner`.
    deferred: Arc<Mutex<Deferred>>,
}

/// The most events held back while the manager lock is busy. Past this the
/// events are counted, not kept, and the loss is made explicit by marking
/// the projects `Desynced`.
const MAX_DEFERRED_EVENTS: usize = 4096;

#[derive(Default)]
struct Deferred {
    events: Vec<Event>,
    /// Events that did not fit under `MAX_DEFERRED_EVENTS`.
    overflowed: u64,
}

#[derive(Default)]
struct WatchManagerInner {
    /// Per-project watch state and derived scope/rules, keyed by
    /// canonical project root.
    projects: HashMap<PathBuf, ProjectWatchInfo>,
    /// The underlying notify watcher. May be absent if the platform
    /// doesn't support filesystem watching.
    watcher: Option<RecommendedWatcher>,
}

/// Everything the manager tracks for one active project: the watch state,
/// the current scope/rules, and a flag that says an ignore file changed
/// and the scope must be recomputed on the next tool-thread pass.
#[derive(Default)]
struct ProjectWatchInfo {
    state: WatchState,
    /// The last computed scope. `None` until the first computation.
    scope: Option<WatchScope>,
    /// The rules built from the scope's ignore files.
    rules: Option<Arc<IgnoreRules>>,
    /// Set when an event for a `.gitignore`/`.ignore` file was dropped in
    /// — the next `refresh_scope` from a tool thread recomputes scope and
    /// rules before anything else is reconciled.
    needs_scope_refresh: bool,
}

impl WatchManager {
    /// Creates a `WatchManager` with **no** `notify` watcher running — the
    /// default for tests; `with_watcher` opts in.
    pub fn new() -> Self {
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
            move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        manager_clone.handle_event(event);
                    }
                    Err(error) => {
                        // Watcher error (e.g., queue overflow, watch removed).
                        // Mark all projects as Desynced so the next ensure_current
                        // does a full verification.
                        tracing::warn!(
                            error = %error,
                            "filesystem watcher error; marking all projects for re-verification"
                        );
                        let guard = lock_or_recover(&manager_clone.inner);
                        for info in guard.projects.values() {
                            info.state.set_health(WatcherHealth::Desynced);
                        }
                    }
                }
            },
            Config::default(),
        ) {
            Ok(watcher) => {
                let mut guard = lock_or_recover(&manager.inner);
                guard.watcher = Some(watcher);
            }
            Err(error) => {
                // The cause is carried, not just the consequence: inotify
                // exhaustion, a permissions problem and a missing kernel
                // feature are indistinguishable without it.
                tracing::warn!(
                    error = %error,
                    "filesystem watcher unavailable; SlugAudit will use full verification"
                );
            }
        }

        manager
    }

    /// Start watching `root`. Returns the `WatchState` for the project.
    /// If the watcher is unavailable, returns a state with `Unavailable`
    /// health. Computes the watch scope immediately and prunes watches on
    /// ignored/excluded directories.
    pub fn watch(&self, root: &Path) -> WatchState {
        let canonical = crate::util::canonical_or_self(root);

        let mut guard = lock_or_recover(&self.inner);
        self.apply_deferred(&mut guard);

        // If we already have a state for this project, return it.
        if let Some(info) = guard.projects.get(&canonical) {
            return info.state.clone();
        }

        let state = WatchState::new();

        // Try to add the watch.
        if let Some(ref mut watcher) = guard.watcher {
            match watcher.watch(&canonical, RecursiveMode::Recursive) {
                Ok(()) => {
                    tracing::info!(root_id = %redacted_path(&canonical), "watching project");
                }
                Err(error) => {
                    tracing::warn!(
                        root_id = %redacted_path(&canonical),
                        error = %error,
                        "failed to watch project root; using Unavailable health"
                    );
                    state.set_health(WatcherHealth::Unavailable);
                }
            }
        } else {
            state.set_health(WatcherHealth::Unavailable);
        }

        // After restart, we don't trust the watcher history. The sync layer
        // must verify the current filesystem state before serving evidence.
        // But if the watcher is Unavailable, we must not overwrite that —
        // Unavailable means every call must do full verification, and setting
        // NeedsVerification here would let the next call incorrectly transition
        // to Healthy after a single full publish.
        if state.health() != WatcherHealth::Unavailable {
            state.set_health(WatcherHealth::NeedsVerification);
        }

        guard.projects.insert(
            canonical.clone(),
            ProjectWatchInfo {
                state: state.clone(),
                ..ProjectWatchInfo::default()
            },
        );
        // Compute the watch scope and prune watches on ignored/excluded
        // directories now, so the watcher and the index agree from the
        // start.
        Self::refresh_scope_locked(&mut guard, &canonical);
        // Events that arrived during the scope walk above.
        self.apply_deferred(&mut guard);
        state
    }

    /// Applies the events deferred while the lock was busy, oldest first,
    /// through the same path as a live event. Events past
    /// `MAX_DEFERRED_EVENTS` were not kept, so that loss is made explicit:
    /// every project that is not `Unavailable` becomes `Desynced` and the
    /// sync that follows does a full verification.
    fn apply_deferred(&self, inner: &mut WatchManagerInner) {
        let Deferred { events, overflowed } = std::mem::take(&mut *lock_or_recover(&self.deferred));
        if overflowed > 0 {
            tracing::warn!(
                overflowed,
                kept = events.len(),
                "too many filesystem events arrived while the watch manager was busy; \
                 marking projects for full re-verification"
            );
            for info in inner.projects.values() {
                if info.state.health() != WatcherHealth::Unavailable {
                    info.state.set_health(WatcherHealth::Desynced);
                }
            }
        }
        if !events.is_empty() {
            tracing::debug!(
                count = events.len(),
                "applying filesystem events deferred while the watch manager was busy"
            );
        }
        for event in &events {
            Self::apply_event(inner, event);
        }
    }

    /// Stop watching `root` and remove its state.
    pub fn unwatch(&self, root: &Path) {
        let canonical = crate::util::canonical_or_self(root);

        let mut guard = lock_or_recover(&self.inner);

        // The project is dropped from `projects` either way, so a failed
        // unwatch leaves the watcher delivering events for a project this
        // manager no longer tracks.
        if let Some(ref mut watcher) = guard.watcher
            && let Err(error) = watcher.unwatch(&canonical)
        {
            tracing::warn!(root_id = %redacted_path(&canonical), error = %error, "unwatch failed");
        }

        guard.projects.remove(&canonical);
    }

    /// Get the `WatchState` for `root`, if it exists.
    pub fn get(&self, root: &Path) -> Option<WatchState> {
        let canonical = crate::util::canonical_or_self(root);
        let guard = lock_or_recover(&self.inner);
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
        let guard = lock_or_recover(&self.inner);
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
        let guard = lock_or_recover(&self.inner);
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
        let canonical = crate::util::canonical_or_self(root);
        let mut guard = lock_or_recover(&self.inner);
        // Before the check: a deferred ignore-file event is what sets
        // `needs_scope_refresh`.
        self.apply_deferred(&mut guard);
        let needs = guard
            .projects
            .get(&canonical)
            .map(|info| info.scope.is_none() || info.needs_scope_refresh)
            .unwrap_or(false);
        if needs {
            Self::refresh_scope_locked(&mut guard, &canonical);
        }
    }

    /// The `IgnoreRules` currently in effect for `root`, if the project
    /// is registered. Used by the sync layer to filter dirty paths before
    /// indexing them.
    pub fn rules_for(&self, root: &Path) -> Option<Arc<IgnoreRules>> {
        let canonical = crate::util::canonical_or_self(root);
        let guard = lock_or_recover(&self.inner);
        guard
            .projects
            .get(&canonical)
            .and_then(|info| info.rules.clone())
    }

    /// Recomputes the scope for `root`, then reconciles the actual
    /// watches with it: unwatch directories the walker now prunes, watch
    /// directories it now includes. If the watch set changed, the project
    /// is marked `NeedsVerification` so the next sync does a full publish
    /// — files may have been created or modified inside re-added
    /// directories while they were unwatched, and newly-ignored files may
    /// still be in the database.
    ///
    /// Must hold the manager lock. Must run on a tool thread (see
    /// `refresh_scope`).
    fn refresh_scope_locked(guard: &mut MutexGuard<'_, WatchManagerInner>, root: &Path) {
        let (old_scope, scope, rules) = {
            let info = match guard.projects.get_mut(root) {
                Some(info) => info,
                None => return,
            };
            let scope = WatchScope::compute(root);
            let rules = Arc::new(IgnoreRules::build(root, &scope.ignore_files));
            (info.scope.clone(), scope, rules)
        };

        let (new_watches, removed_watches) = watch_dir_changes(old_scope.as_ref(), &scope, root);

        let scope_changed = !new_watches.is_empty() || !removed_watches.is_empty();
        if let Some(watcher) = guard.watcher.as_mut() {
            // Both arms log. A failure here was previously invisible: the
            // debug line sat inside the `is_ok()` branch, so a watch that
            // could not be established left no trace at any level while the
            // project was still marked `NeedsVerification` below as though
            // the watch set had been applied.
            // A failed unwatch leaves the watcher delivering events for files
            // the scope no longer includes.
            for dir in &removed_watches {
                if let Err(error) = watcher.unwatch(dir) {
                    tracing::warn!(
                        dir_id = %redacted_path(dir), error = %error, "unwatch failed"
                    );
                }
            }
            // A failed watch means changes under that directory go unnoticed
            // until a full verification.
            for dir in &new_watches {
                if let Err(error) = watcher.watch(dir, RecursiveMode::Recursive) {
                    tracing::warn!(dir_id = %redacted_path(dir), error = %error, "watch failed");
                }
            }
        }
        // With no notify watcher (unsupported platform or tests) there is
        // nothing to apply here, but the scope and rules are recorded above
        // either way so event filtering and reconcile can use them.

        if let Some(info) = guard.projects.get_mut(root) {
            info.scope = Some(scope);
            info.rules = Some(rules);
            info.needs_scope_refresh = false;
            if scope_changed && info.state.health() != WatcherHealth::Unavailable {
                info.state.set_health(WatcherHealth::NeedsVerification);
            }
        }
    }

    /// Handle a `notify` event: normalize the path and mark it dirty/deleted,
    /// unless the project's rules exclude it. Uses `try_lock`, never `lock`:
    /// `unwatch` runs under the manager lock on a tool thread and blocks
    /// waiting for this very notify thread, so blocking here would deadlock.
    /// An event that finds the lock held is not dropped: it is deferred and
    /// applied, in order, by the next thread that holds the lock (this
    /// handler on its next event, or a tool thread in `watch`/`refresh_scope`
    /// before the sync reads the dirty sets).
    fn handle_event(&self, event: Event) {
        let mut guard = match self.inner.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                self.defer(event);
                return;
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                tracing::error!("mutex was poisoned; recovering inner value");
                poisoned.into_inner()
            }
        };
        self.apply_deferred(&mut guard);
        Self::apply_event(&mut guard, &event);

        // Check if any project roots have been deleted (e.g. tempdirs
        // dropped after tests). If so, unwatch and remove them to prevent
        // the map from accumulating stale entries.
        let deleted_roots: Vec<PathBuf> = guard
            .projects
            .keys()
            .filter(|root| !root.exists())
            .cloned()
            .collect();

        // Deliberately no `watcher.unwatch(root)` here: this loop runs on
        // the notify event-loop thread, and notify-rs's `unwatch` blocks
        // waiting for a response from that same loop - calling it from
        // inside a callback deadlocks (the loop can't service the request
        // while it's inside the handler). It's also unnecessary: the
        // kernel auto-removes the watch when the watched directory is
        // deleted (notify's Linux backend is inotify), so notify already
        // treats the watch as gone. `unwatch` remains safe from
        // `WatchManager::unwatch` and `refresh_scope_locked`, which the
        // sync layer calls from a tool thread, never from the event loop.
        for root in &deleted_roots {
            guard.projects.remove(root);
            tracing::info!(root_id = %redacted_path(root), "removed watch for deleted project root");
        }
    }

    /// Holds `event` back until a thread that owns the manager lock applies
    /// it. Never blocks on `inner`; `deferred` is only ever held for a push
    /// or a take, so the notify thread cannot wait on a tool thread here.
    fn defer(&self, event: Event) {
        let mut deferred = lock_or_recover(&self.deferred);
        if deferred.events.len() < MAX_DEFERRED_EVENTS {
            deferred.events.push(event);
        } else {
            deferred.overflowed += 1;
        }
    }

    /// Routes one event to the project it belongs to and records it.
    fn apply_event(inner: &mut WatchManagerInner, event: &Event) {
        for path in &event.paths {
            for (project_root, info) in inner.projects.iter_mut() {
                if let Some(relative) = normalize_relative_path(project_root, path) {
                    match classify_path(&relative, info.rules.as_deref()) {
                        PathClass::Excluded | PathClass::Ignored => continue,
                        PathClass::IgnoreFile => info.needs_scope_refresh = true,
                        PathClass::Tracked => {}
                    }
                    match event.kind {
                        EventKind::Remove(_) => {
                            info.state.mark_deleted(relative);
                        }
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            info.state.mark_dirty(relative);
                        }
                        _ => {}
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "manager_event_tests.rs"]
mod event_tests;
