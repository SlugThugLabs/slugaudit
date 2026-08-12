// slugaudit-line-exception: approved-by=agent; reason=one file owns the notify watcher lifecycle, per-project watch states, scope/rule maintenance, and the event filter; splitting would fragment the manager's lock discipline and the tool-thread-vs-event-thread unwatch rule across modules
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

use super::path::normalize_relative_path;
use super::scope::WatchScope;
use super::state::WatchState;
use super::types::WatcherHealth;
use crate::ignore_rules::{IgnoreRules, is_excluded_path};
use crate::util::lock_or_recover;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Manages filesystem watchers for all active projects. One instance lives
/// on the `SlugAuditServer` (or its sync manager). Each project gets its
/// own `WatchState`; the manager routes `notify` events to the right state.
#[derive(Clone, Default)]
pub struct WatchManager {
    inner: Arc<Mutex<WatchManagerInner>>,
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
            Err(_) => {
                tracing::warn!(
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
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

        let mut guard = lock_or_recover(&self.inner);

        // If we already have a state for this project, return it.
        if let Some(info) = guard.projects.get(&canonical) {
            return info.state.clone();
        }

        let state = WatchState::new();

        // Try to add the watch.
        if let Some(ref mut watcher) = guard.watcher {
            match watcher.watch(&canonical, RecursiveMode::Recursive) {
                Ok(()) => {
                    tracing::info!(root = %canonical.display(), "watching project");
                }
                Err(error) => {
                    tracing::warn!(
                        root = %canonical.display(),
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
        state
    }

    /// Stop watching `root` and remove its state.
    pub fn unwatch(&self, root: &Path) {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

        let mut guard = lock_or_recover(&self.inner);

        if let Some(ref mut watcher) = guard.watcher {
            let _ = watcher.unwatch(&canonical);
        }

        guard.projects.remove(&canonical);
    }

    /// Get the `WatchState` for `root`, if it exists.
    pub fn get(&self, root: &Path) -> Option<WatchState> {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
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
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut guard = lock_or_recover(&self.inner);
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
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
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

        let (new_watches, removed_watches) = match &old_scope {
            Some(old) => (
                scope
                    .watch_dirs
                    .difference(&old.watch_dirs)
                    .cloned()
                    .collect::<Vec<_>>(),
                old.watch_dirs
                    .difference(&scope.watch_dirs)
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            // First scope: the recursive root watch registered above is
            // the only thing to prune down to the indexable set.
            None => (
                scope
                    .watch_dirs
                    .iter()
                    .filter(|dir| *dir != root)
                    .cloned()
                    .collect::<Vec<_>>(),
                Vec::new(),
            ),
        };

        let scope_changed = !new_watches.is_empty() || !removed_watches.is_empty();
        if let Some(watcher) = guard.watcher.as_mut() {
            for dir in &removed_watches {
                if watcher.unwatch(dir).is_ok() {
                    tracing::debug!(dir = %dir.display(), "unwatched excluded directory");
                }
            }
            for dir in &new_watches {
                if watcher.watch(dir, RecursiveMode::Recursive).is_ok() {
                    tracing::debug!(dir = %dir.display(), "watching indexable directory");
                }
            }
        } else {
            // No notify watcher (unsupported platform or tests): the scope
            // and rules are still recorded so event filtering and
            // reconcile use them.
        }

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
    /// unless the project's rules exclude it. Uses `try_lock` to avoid
    /// blocking the watcher's background thread if the sync layer holds the
    /// lock — stale events are harmless because the sync layer will do a
    /// full verification on the next `ensure_current`.
    fn handle_event(&self, event: Event) {
        let paths: Vec<PathBuf> = event.paths.clone();

        let Ok(mut guard) = self.inner.try_lock() else {
            // Sync layer holds the lock — skip this event. The sync layer
            // will do a full verification on the next `ensure_current`, so
            // missing an event here is harmless.
            return;
        };

        // Check if any project roots have been deleted (e.g. tempdirs
        // dropped after tests). If so, unwatch and remove them to prevent
        // the map from accumulating stale entries.
        let deleted_roots: Vec<PathBuf> = guard
            .projects
            .keys()
            .filter(|root| !root.exists())
            .cloned()
            .collect();

        for path in &paths {
            for (project_root, info) in guard.projects.iter_mut() {
                if let Some(relative) = normalize_relative_path(project_root, path) {
                    if is_excluded_path(&relative) {
                        continue;
                    }
                    if is_ignore_file(&relative) {
                        // Ignore files change the watch scope itself
                        // (directories can become indexable or not). Flag
                        // the project so the next tool-thread pass
                        // recomputes the scope and rules before
                        // reconciling anything.
                        info.needs_scope_refresh = true;
                    } else if let Some(rules) = &info.rules
                        && rules.should_ignore(&relative)
                    {
                        // A gitignored path: never mark dirty. Reconcile
                        // applies the same filter, so this is belt and
                        // braces — but a full publish skips these files,
                        // so the incremental path must too.
                        continue;
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

        // Deliberately no `watcher.unwatch(root)` here: this loop runs on
        // the notify event-loop thread, and notify-rs's `unwatch` blocks
        // waiting for a response from that same loop — calling it from
        // inside a callback deadlocks (the loop can't service the request
        // while it's inside the handler). It's also unnecessary: the
        // kernel auto-removes the watch when the watched directory is
        // deleted (notify's Linux backend is inotify), so notify already
        // treats the watch as gone. `unwatch` remains safe from
        // `WatchManager::unwatch` and `refresh_scope_locked`, which the
        // sync layer calls from a tool thread, never from the event loop.
        for root in &deleted_roots {
            guard.projects.remove(root);
            tracing::info!(root = %root.display(), "removed watch for deleted project root");
        }
    }
}

/// True if `relative` names a `.gitignore` or `.ignore` file — an event
/// on these must trigger a scope refresh, since the rules they carry can
/// change which directories are watched and indexed.
fn is_ignore_file(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    name == ".gitignore" || name == ".ignore"
}

#[cfg(test)]
#[path = "manager_event_tests.rs"]
mod event_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_excluded_path_filters_slugaudit_and_scratch_files() {
        assert!(is_excluded_path(".planning/slugaudit/project.db"));
        assert!(is_excluded_path(".git/config"));
        assert!(is_excluded_path("src/lib.rs~"));
        assert!(is_excluded_path("notes.tmp"));
        assert!(!is_excluded_path("src/lib.rs"));
        assert!(!is_excluded_path("README.md"));
    }

    #[test]
    fn is_ignore_file_detects_ignore_files_at_any_depth() {
        assert!(is_ignore_file(".gitignore"));
        assert!(is_ignore_file("sub/.gitignore"));
        assert!(is_ignore_file(".ignore"));
        assert!(is_ignore_file("sub/deep/.ignore"));
        assert!(!is_ignore_file("src/lib.rs"));
        assert!(!is_ignore_file("gitignore"));
    }
}
