//! Applying a computed watch scope while the manager lock is held.
//!
//! Split from `scope.rs` so the pure walk and the lock-held watch-set diff
//! each stay within the production line limit. The expensive walk and rules
//! build happen outside the lock; everything here runs under it and is the
//! only part that touches the `notify` watcher.

use super::event_filter::watch_dir_changes;
use super::manager::{ProjectWatchInfo, WatchManagerInner};
use super::scope::WatchScope;
use super::types::WatcherHealth;
use crate::ignore_rules::IgnoreRules;
use crate::watch::errors::log_notify_failure;
use notify::{RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;

/// Applies a precomputed scope and ignore rules to `root` while the manager
/// lock is held: diffs the watch set, runs the watcher's `unwatch`/`watch`
/// calls (which must happen on a tool thread under this lock), records the
/// scope and rules, and moves the health between `Healthy` and `Partial`
/// when the scope's limitation changed.
///
/// - A changed watch set still forces `NeedsVerification`: files may have
///   been created or modified inside re-added directories while they were
///   unwatched, and newly-ignored files may still be in the database.
/// - `Unavailable` is never overwritten: every call must keep full-verifying.
/// - `needs_scope_refresh` is cleared only when no ignore-file event landed
///   while the walk was running (`scope_generation` unchanged); otherwise it
///   stays set so the next tool-thread pass recomputes instead of swallowing
///   a rule change the walk may not have seen.
pub(super) fn apply_scope_locked(
    inner: &mut WatchManagerInner,
    root: &Path,
    scope: WatchScope,
    rules: Arc<IgnoreRules>,
    generation_before: u64,
) {
    let Some(info) = inner.projects.get(root) else {
        return;
    };
    let (new_watches, removed_watches) = watch_dir_changes(info.scope.as_ref(), &scope, root);
    let scope_changed = !new_watches.is_empty() || !removed_watches.is_empty();
    let limited = scope.limit.is_some();

    if let Some(limit) = scope.limit.as_ref() {
        tracing::warn!(
            root = %root.display(),
            max_entries = limit.max_entries,
            max_depth = limit.max_depth,
            entries_seen = limit.entries_seen,
            depth_limited = limit.depth_limited,
            "watch scope truncated at a documented limit; directories beyond it are \
             unwatched, and the stat sweep remains the freshness guarantee",
        );
    }

    if let Some(watcher) = inner.watcher.as_mut() {
        // Both arms log. A failure here was previously invisible: the debug
        // line sat inside the `is_ok()` branch, so a watch that could not be
        // established left no trace at any level.
        // A failed unwatch leaves the watcher delivering events for files
        // the scope no longer includes.
        for dir in &removed_watches {
            if let Err(error) = watcher.unwatch(dir) {
                log_notify_failure("scope_unwatch", &dir.display().to_string(), &error);
            }
        }
        // A failed watch means changes under that directory go unnoticed
        // until a full verification. Non-recursive: every directory the scope
        // includes is registered individually, so the number of kernel
        // watches is bounded by the scope walk's entry cap instead of by the
        // whole subtree notify would otherwise walk (ignored trees included).
        for dir in &new_watches {
            if let Err(error) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                log_notify_failure("scope_watch", &dir.display().to_string(), &error);
            }
        }
    }
    // With no notify watcher (unsupported platform or tests) there is
    // nothing to apply here, but the scope and rules are recorded either way
    // so event filtering and reconcile can use them.

    let Some(info) = inner.projects.get_mut(root) else {
        return;
    };
    let health = info.state.health();
    info.scope = Some(scope);
    info.rules = Some(rules);
    info.needs_scope_refresh = info.scope_generation != generation_before;
    if scope_changed && health != WatcherHealth::Unavailable {
        info.state.set_health(WatcherHealth::NeedsVerification);
    } else if health == WatcherHealth::Healthy && limited {
        info.state.set_health(WatcherHealth::Partial);
    } else if health == WatcherHealth::Partial && !limited {
        info.state.set_health(WatcherHealth::Healthy);
    }
}

/// True when the project registered under `root` has a scope that stopped
/// at a documented limit. The manager lock must be held by the caller;
/// [`super::WatchManager::scope_limited`] is the tool-facing entry point.
pub(super) fn scope_limited(inner: &WatchManagerInner, root: &Path) -> bool {
    inner
        .projects
        .get(root)
        .and_then(|info| info.scope.as_ref())
        .is_some_and(|scope| scope.limit.is_some())
}

impl ProjectWatchInfo {
    /// Marks the scope stale for the next tool-thread pass. The generation
    /// lets a walk that started before this call detect that the ignore
    /// rules it read may already be obsolete, so it leaves the refresh
    /// pending instead of clearing it.
    pub(super) fn mark_scope_dirty(&mut self) {
        self.needs_scope_refresh = true;
        // Wrapping is intentional: 2^64 ignore-file events cannot occur in
        // a process lifetime, and a panic on overflow would fail a tool call.
        self.scope_generation = self.scope_generation.wrapping_add(1);
    }
}
