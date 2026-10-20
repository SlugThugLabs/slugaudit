//! Lock-held event dispatch for [`super::WatchManager`].
//!
//! Split from `manager.rs` to keep the manager within the production line
//! limit: the notify callback's `try_lock`/defer path, replay of deferred
//! events, and per-project event application all live here. They are
//! methods of `WatchManager` so the manager's one-lock rule (and the rule
//! that `unwatch` only ever runs on a tool thread) stays intact.

use super::{Deferred, MAX_DEFERRED_EVENTS, WatchManager, WatchManagerInner};
use crate::util::{lock_or_recover, redacted_path};
use crate::watch::event_filter::{PathClass, classify_path};
use crate::watch::path::normalize_relative_path;
use crate::watch::types::WatcherHealth;
use notify::{Event, EventKind};
use std::path::PathBuf;
use std::sync::TryLockError;

impl WatchManager {
    /// Applies the events deferred while the lock was busy, oldest first,
    /// through the same path as a live event. Events past
    /// `MAX_DEFERRED_EVENTS` were not kept, so that loss is made explicit:
    /// every project that is not `Unavailable` becomes `Desynced` and the
    /// sync that follows does a full verification.
    pub(super) fn apply_deferred(&self, inner: &mut WatchManagerInner) {
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

    /// Handle a `notify` event: normalize the path and mark it dirty/deleted,
    /// unless the project's rules exclude it. Uses `try_lock`, never `lock`:
    /// `unwatch` runs under the manager lock on a tool thread and blocks
    /// waiting for this very notify thread, so blocking here would deadlock.
    /// An event that finds the lock held is not dropped: it is deferred and
    /// applied, in order, by the next thread that holds the lock (this
    /// handler on its next event, or a tool thread in `watch`/`refresh_scope`
    /// before the sync reads the dirty sets).
    pub(super) fn handle_event(&self, event: Event) {
        let mut guard = match self.inner.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                self.defer(event);
                return;
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                // The event covers every project, so no single root exists.
                let root_id = "unscoped";
                tracing::error!(operation = "handle_event_lock_recovery", mutex = "watch_manager_inner", root_id = %root_id, "watch manager mutex was poisoned; recovering the inner value");
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
        // `WatchManager::unwatch` and `scope::apply_scope_locked`, which the
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
                        PathClass::IgnoreFile => info.mark_scope_dirty(),
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
