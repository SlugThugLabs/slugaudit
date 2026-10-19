//! Watch-scope computation: which directories under a project root should
//! carry filesystem watches.
//!
//! Discovery indexes only the files its walker yields (honoring ignore
//! files, VCS internals, and scratch-file rules), but the watcher used to
//! register a single recursive watch over the whole project root — every
//! directory, including `target/`, `node_modules/`, and `.git/`. That
//! consumed a kernel watch descriptor per directory for trees whose
//! contents are never indexed, and the incremental path could even index
//! files the walker would skip.
//!
//! This module enumerates the *indexable* directory set with the very
//! same walker discovery uses, so the watch scope and the index scope
//! agree by construction.
//!
//! The walk is bounded (`max_watch_entries` / `max_watch_depth`): an
//! unbounded walk over a pathological tree would hold the tool thread for
//! as long as the tree takes and retain a path per directory. A walk that
//! stops at a limit records it in [`WatchScope::limit`], the manager
//! reports `WatcherHealth::Partial`, and the per-call stat sweep remains
//! the freshness guarantee for the directories beyond the limit.
//!
//! The scope and the rules built from it are computed **outside** the
//! manager lock; [`apply_scope_locked`] runs under it and is the only part
//! that touches the watcher. That split is what keeps a large walk from
//! blocking every tool call and the notify callback.

use super::event_filter::watch_dir_changes;
use super::manager::{ProjectWatchInfo, WatchManagerInner};
use super::types::WatcherHealth;
use crate::ignore_rules::{
    IgnoreRules, indexable_walker_with_max_depth, is_excluded_dir, is_excluded_path,
};
use notify::{RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A documented limit at which a scope walk stopped. Present on a
/// [`WatchScope`] exactly when the walk is incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchScopeLimit {
    /// The entry cap the walk was given.
    pub max_entries: usize,
    /// The depth cap the walk was given (the root is depth 0).
    pub max_depth: usize,
    /// Walker entries yielded before the walk stopped. Equal to
    /// `max_entries` exactly when the entry cap stopped it.
    pub entries_seen: usize,
    /// True when a directory at `max_depth` had children the walker could
    /// not descend into. The depth cap can be hit with the entry count still
    /// below its cap.
    pub depth_limited: bool,
}

/// The set of directories that should carry filesystem watches, plus the
/// ignore files found inside them (used to build `IgnoreRules`).
#[derive(Debug, Clone, Default)]
pub struct WatchScope {
    /// Absolute paths of the directories to watch: everything the
    /// indexable walker visits that isn't VCS internals or SlugAudit's
    /// own data directory. The project root is always included.
    pub watch_dirs: HashSet<PathBuf>,
    /// The `.gitignore`/`.ignore` files discovered, sorted, for building
    /// the event-filtering matcher.
    pub ignore_files: Vec<PathBuf>,
    /// Present when the walk stopped at `max_watch_entries` or
    /// `max_watch_depth`; `None` means the scope covers the whole
    /// indexable tree.
    pub limit: Option<WatchScopeLimit>,
}

impl WatchScope {
    /// Computes the scope with the process-wide watch limits
    /// (`max_watch_entries` / `max_watch_depth`). If the root can't be
    /// walked at all, the returned scope contains only the root itself.
    pub fn compute(root: &Path) -> WatchScope {
        let limits = crate::model::process_limits();
        Self::compute_with_limits(root, limits.max_watch_entries, limits.max_watch_depth)
    }

    /// Computes the scope under explicit caps. Every yielded walker entry
    /// counts against `max_entries`; the walk stops before processing an
    /// entry past the cap, so a tree exactly at the cap is not reported as
    /// truncated. `max_depth` bounds descent, with the root at depth 0.
    ///
    /// The project root is always in `watch_dirs`, even when both caps are
    /// zero, so the manager always has a directory it can watch.
    pub fn compute_with_limits(root: &Path, max_entries: usize, max_depth: usize) -> WatchScope {
        #[cfg(test)]
        super::run_walk_hook();

        let mut scope = WatchScope::default();
        let mut entries_seen = 0usize;
        let mut entries_limited = false;
        let mut depth_limited = false;

        for entry in indexable_walker_with_max_depth(root, Some(max_depth)) {
            // Another entry exists past the cap: the walk is truncated.
            // Checked before counting, so exactly-at-cap is not limited and
            // at most `max_entries` entries are processed.
            if entries_seen >= max_entries {
                entries_limited = true;
                break;
            }
            entries_seen += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    // The doc above covers only "the root can't be walked at
                    // all". A failing subtree under-registers watches, which
                    // means changes under it are not seen until a full
                    // verification — worth a line rather than silence.
                    tracing::warn!(
                        error = %error,
                        "watch scope could not read a directory entry; that subtree is \
                         unwatched until the next scope refresh",
                    );
                    continue;
                }
            };
            let relative = match entry.path().strip_prefix(root) {
                Ok(relative) => relative,
                Err(error) => {
                    // The walker yields paths under `root`, so this cannot
                    // happen; if it did, the entry would go unwatched.
                    tracing::warn!(
                        path_id = %crate::util::redacted_path(entry.path()),
                        error = %error,
                        "watch scope walked a path outside the project root; it is left out",
                    );
                    continue;
                }
            };
            let relative_str = relative.to_string_lossy();
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if is_excluded_dir(&relative_str) {
                    continue;
                }
                // A directory *at* the depth cap is yielded but never
                // descended into. If it has any child, that child is
                // unwalked because of the cap.
                if relative.components().count() == max_depth {
                    match directory_has_entries(entry.path()) {
                        Ok(has_entries) => depth_limited |= has_entries,
                        Err(error) => {
                            // Can't prove it is empty: fail closed and report
                            // the scope as partial rather than claim full
                            // coverage over children we could not inspect.
                            depth_limited = true;
                            tracing::warn!(
                                path_id = %crate::util::redacted_path(entry.path()),
                                error = %error,
                                "could not inspect a directory at the watch-scope depth \
                                 limit; treating its children as unwatched",
                            );
                        }
                    }
                }
                scope.watch_dirs.insert(entry.path().to_path_buf());
            } else if file_type.is_file() {
                if is_excluded_path(&relative_str) {
                    continue;
                }
                let name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == ".gitignore" || name == ".ignore" {
                    scope.ignore_files.push(entry.path().to_path_buf());
                }
            }
        }
        scope.ignore_files.sort();
        scope.watch_dirs.insert(root.to_path_buf());
        if entries_limited || depth_limited {
            scope.limit = Some(WatchScopeLimit {
                max_entries,
                max_depth,
                entries_seen,
                depth_limited,
            });
        }
        scope
    }
}

/// True when `path` has at least one directory entry. An entry that could
/// not even be read counts as present: a directory we can't inspect may
/// hold unwatched children, and the caller reports that as a partial scope.
fn directory_has_entries(path: &Path) -> std::io::Result<bool> {
    let mut entries = std::fs::read_dir(path)?;
    Ok(entries.next().is_some())
}

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
            root_id = %crate::util::redacted_path(root),
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
                tracing::warn!(
                    dir_id = %crate::util::redacted_path(dir), error = %error, "unwatch failed"
                );
            }
        }
        // A failed watch means changes under that directory go unnoticed
        // until a full verification.
        for dir in &new_watches {
            if let Err(error) = watcher.watch(dir, RecursiveMode::Recursive) {
                tracing::warn!(dir_id = %crate::util::redacted_path(dir), error = %error, "watch failed");
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

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
