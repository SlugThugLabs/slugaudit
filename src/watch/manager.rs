//! Filesystem watcher manager. Owns the `notify` watcher and dispatches
//! events to per-project `WatchState`. Runs on a dedicated background
//! thread; the MCP server interacts with it only through this manager.

use super::path::normalize_relative_path;
use super::state::WatchState;
use super::types::WatcherHealth;
use crate::util::lock_or_recover;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Manages filesystem watchers for all active projects. One instance lives
/// on the `SlugAuditServer` (or its sync manager). Each project gets its
/// own `WatchState`; the manager routes `notify` events to the right state.
#[derive(Clone, Default)]
pub struct WatchManager {
    inner: Arc<Mutex<WatchManagerInner>>,
}

#[derive(Default)]
struct WatchManagerInner {
    /// Per-project watch state, keyed by canonical project root.
    projects: HashMap<PathBuf, WatchState>,
    /// The underlying notify watcher. May be absent if the platform
    /// doesn't support filesystem watching.
    watcher: Option<RecommendedWatcher>,
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
                        for state in guard.projects.values() {
                            state.set_health(WatcherHealth::Desynced);
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
    /// health.
    pub fn watch(&self, root: &Path) -> WatchState {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

        let mut guard = lock_or_recover(&self.inner);

        // If we already have a state for this project, return it.
        if let Some(state) = guard.projects.get(&canonical) {
            return state.clone();
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

        guard.projects.insert(canonical.clone(), state.clone());
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
        guard.projects.get(&canonical).cloned()
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
            .map(|(root, state)| (root.clone(), state.clone()))
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
            .map(|state| state.snapshot())
            .collect()
    }

    /// Handle a `notify` event: normalize the path and mark it dirty/deleted.
    /// Uses `try_lock` to avoid blocking the watcher's background thread if
    /// the sync layer holds the lock — stale events are harmless because the
    /// sync layer will do a full verification on the next `ensure_current`.
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
            for (project_root, state) in &guard.projects {
                if let Some(relative) = normalize_relative_path(project_root, path) {
                    if is_excluded_path(&relative) {
                        continue;
                    }
                    match event.kind {
                        EventKind::Remove(_) => {
                            state.mark_deleted(relative);
                        }
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            state.mark_dirty(relative);
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
        // kernel auto-removes the inotify watch when the watched
        // directory is deleted, so notify already treats the watch as
        // gone. `unwatch` remains safe from `WatchManager::unwatch`,
        // which the sync layer calls from a tool thread, never from the
        // event loop.
        for root in &deleted_roots {
            guard.projects.remove(root);
            tracing::info!(root = %root.display(), "removed watch for deleted project root");
        }
    }
}

/// Paths that should be ignored by the watcher, mirroring `sync::discovery`.
fn is_excluded_path(relative: &str) -> bool {
    relative.starts_with(".planning/slugaudit")
        || relative.split('/').any(|component| component == ".git")
        || relative.ends_with(".claude_output.txt")
        || relative.ends_with(".tmp")
        || relative.ends_with(".bak")
        || relative.ends_with(".swp")
        || relative.ends_with('~')
}

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
}
