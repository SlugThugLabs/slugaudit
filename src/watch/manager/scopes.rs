//! Scope registration and refresh for [`super::WatchManager`].
//!
//! Split from `manager.rs` to keep the manager within the production line
//! limit. Both entry points keep the same split as before: the expensive
//! scope walk and `IgnoreRules` build run **outside** the manager lock, and
//! only registration, the refresh decision, and the watch-set diff are
//! serialized by it.

use super::{ProjectWatchInfo, WatchManager};
use crate::ignore_rules::IgnoreRules;
use crate::util::{canonical_or_self, lock_or_recover, redacted_path};
use crate::watch::errors::log_notify_failure;
use crate::watch::scope::{self, WatchScope};
use crate::watch::state::WatchState;
use crate::watch::types::WatcherHealth;
use notify::{RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;

impl WatchManager {
    /// [`Self::watch`] with explicit scope caps (the tests use this to make a
    /// small tree truncate).
    ///
    /// Registration happens under the lock, but the scope walk does not: the
    /// project is inserted first so an ignore-file event arriving during the
    /// walk lands on it and bumps its scope generation, then the walk runs
    /// with no lock held, then the result is applied under the lock.
    pub(in crate::watch) fn watch_with_scope_limits(
        &self,
        root: &Path,
        max_entries: usize,
        max_depth: usize,
    ) -> WatchState {
        let canonical = canonical_or_self(root);

        let state = {
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
                        log_notify_failure("watch_project", &redacted_path(&canonical), &error);
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
                    // The walk below has not produced a scope yet. If a
                    // concurrent `refresh_scope` observes this placeholder,
                    // `scope.is_none()` makes it compute one; this flag also
                    // keeps the state honest for that reader.
                    needs_scope_refresh: true,
                    ..ProjectWatchInfo::default()
                },
            );
            state
        };

        // The walk and the ignore-matcher build are the slow part and run
        // without the lock: tool calls and the notify callback both contend
        // for it. `generation_before` is the placeholder's generation; an
        // ignore-file event applied while we walk bumps it.
        let scope = WatchScope::compute_with_limits(&canonical, max_entries, max_depth);
        let rules = Arc::new(IgnoreRules::build(&canonical, &scope.ignore_files));

        let mut guard = lock_or_recover(&self.inner);
        self.apply_deferred(&mut guard);
        scope::apply_scope_locked(&mut guard, &canonical, scope, rules, 0);
        // Events that arrived while the scope was being applied.
        self.apply_deferred(&mut guard);
        state
    }

    /// [`Self::refresh_scope`] with explicit caps. The expensive half — the
    /// scope walk, which reads the tree and its ignore files, and the
    /// `IgnoreRules` build — runs **outside** the manager lock. Only the
    /// decision to refresh, the generation capture, and the watch-set diff
    /// (including `watcher.watch`/`unwatch`) are serialized by it.
    pub(in crate::watch) fn refresh_scope_with_limits(
        &self,
        root: &Path,
        max_entries: usize,
        max_depth: usize,
    ) {
        let canonical = canonical_or_self(root);

        // Decide under the lock and capture the generation the walk starts
        // from. Before the check: a deferred ignore-file event is what sets
        // `needs_scope_refresh`.
        let generation_before = {
            let mut guard = lock_or_recover(&self.inner);
            self.apply_deferred(&mut guard);
            match guard.projects.get(&canonical) {
                Some(info) if info.scope.is_none() || info.needs_scope_refresh => {
                    info.scope_generation
                }
                _ => return,
            }
        };

        let scope = WatchScope::compute_with_limits(&canonical, max_entries, max_depth);
        let rules = Arc::new(IgnoreRules::build(&canonical, &scope.ignore_files));

        let mut guard = lock_or_recover(&self.inner);
        // Ignore-file events that arrived during the walk bumped the
        // generation; `apply_scope_locked` leaves the refresh pending in
        // that case, so the next tool-thread pass recomputes instead of
        // swallowing a rule change this walk may not have seen.
        self.apply_deferred(&mut guard);
        scope::apply_scope_locked(&mut guard, &canonical, scope, rules, generation_before);
    }
}
