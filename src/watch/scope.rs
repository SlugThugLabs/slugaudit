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
//! Watches are registered one scoped directory at a time, non-recursively:
//! notify's recursive mode walks the whole subtree and adds a kernel watch
//! per directory (ignored trees included), which exhausts inotify regardless
//! of the entry cap. Because each directory in the scope is watched
//! directly, the entry cap is also the kernel-watch bound. A directory
//! created later is detected from its parent's event and added by the next
//! scope refresh.
//!
//! The scope and the rules built from it are computed **outside** the
//! manager lock; [`apply_scope_locked`] runs under it and is the only part
//! that touches the watcher. That split is what keeps a large walk from
//! blocking every tool call and the notify callback.

use super::errors::{ignore_error_kind, ignore_error_path};
#[cfg(test)]
use super::manager::{ProjectWatchInfo, WatchManagerInner};
pub(super) use super::scope_apply::{apply_scope_locked, scope_limited};
#[cfg(test)]
use crate::ignore_rules::IgnoreRules;
use crate::ignore_rules::{indexable_walker_with_max_depth, is_excluded_dir, is_excluded_path};
#[cfg(test)]
use notify::Watcher;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;

/// A documented limit at which a scope walk stopped. Present on a
/// [`WatchScope`] exactly when the walk is incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WatchScopeLimit {
    /// The entry cap the walk was given.
    pub(crate) max_entries: usize,
    /// The depth cap the walk was given (the root is depth 0).
    pub(crate) max_depth: usize,
    /// Walker entries yielded before the walk stopped. Equal to
    /// `max_entries` exactly when the entry cap stopped it.
    pub(crate) entries_seen: usize,
    /// True when a directory at `max_depth` had children the walker could
    /// not descend into. The depth cap can be hit with the entry count still
    /// below its cap.
    pub(crate) depth_limited: bool,
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
    pub(crate) limit: Option<WatchScopeLimit>,
}

impl WatchScope {
    /// Computes the scope with the watch limits in `limits`
    /// (`max_watch_entries` / `max_watch_depth`). If the root can't be
    /// walked at all, the returned scope contains only the root itself.
    pub fn compute(root: &Path, limits: &crate::model::ResourceLimits) -> WatchScope {
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

        let mut walk = ScopeWalk::default();
        for entry in indexable_walker_with_max_depth(root, Some(max_depth)) {
            // Another entry exists past the cap: the walk is truncated.
            // Checked before counting, so exactly-at-cap is not limited and
            // at most `max_entries` entries are processed.
            if walk.entries_seen >= max_entries {
                walk.entries_limited = true;
                break;
            }
            walk.entries_seen += 1;
            walk.visit(root, max_depth, entry);
        }
        walk.finish(root, max_entries, max_depth)
    }
}

/// The state of one bounded scope walk: what has been found so far and
/// whether an entry or depth cap stopped it early.
#[derive(Default)]
struct ScopeWalk {
    scope: WatchScope,
    entries_seen: usize,
    entries_limited: bool,
    depth_limited: bool,
}

impl ScopeWalk {
    /// Classifies one walker entry into the scope. Errors and paths outside
    /// the root are logged and left out of the scope, never treated as fatal.
    fn visit(
        &mut self,
        root: &Path,
        max_depth: usize,
        entry: Result<ignore::DirEntry, ignore::Error>,
    ) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                // The docs on `compute_with_limits` cover only "the root
                // can't be walked at all". A failing subtree
                // under-registers watches, which means changes under it are
                // not seen until a full verification — worth a line rather
                // than silence.
                let path_id = ignore_error_path(&error).map(crate::util::redacted_path);
                tracing::warn!(operation = "watch_scope_walk", error_kind = ignore_error_kind(&error), path_id = ?path_id, "watch scope could not read a directory entry; that subtree is unwatched until the next scope refresh");
                return;
            }
        };
        let Some(relative) = Self::relative_to_root(root, entry.path()) else {
            return;
        };
        let Some(file_type) = entry.file_type() else {
            return;
        };
        if file_type.is_dir() {
            self.visit_dir(entry.path(), relative, max_depth);
        } else if file_type.is_file() {
            self.visit_file(entry.path(), relative);
        }
    }

    /// The walker yields paths under `root`; a path outside it is logged and
    /// left out (it would otherwise go unwatched).
    fn relative_to_root<'a>(root: &Path, path: &'a Path) -> Option<&'a Path> {
        match path.strip_prefix(root) {
            Ok(relative) => Some(relative),
            Err(error) => {
                // The walker yields paths under `root`, so this cannot
                // happen; if it did, the entry would go unwatched.
                tracing::warn!(
                    path_id = %crate::util::redacted_path(path),
                    error = %error,
                    "watch scope walked a path outside the project root; it is left out",
                );
                None
            }
        }
    }

    fn visit_dir(&mut self, path: &Path, relative: &Path, max_depth: usize) {
        let relative_str = relative.to_string_lossy();
        if is_excluded_dir(&relative_str) {
            return;
        }
        // A directory *at* the depth cap is yielded but never descended
        // into. If it has any child, that child is unwalked because of the
        // cap.
        if relative.components().count() == max_depth {
            self.note_depth_children(path);
        }
        self.scope.watch_dirs.insert(path.to_path_buf());
    }

    fn note_depth_children(&mut self, path: &Path) {
        match directory_has_entries(path) {
            Ok(has_entries) => self.depth_limited |= has_entries,
            Err(error) => {
                // Can't prove it is empty: fail closed and report the scope
                // as partial rather than claim full coverage over children
                // we could not inspect.
                self.depth_limited = true;
                tracing::warn!(
                    path_id = %crate::util::redacted_path(path),
                    error = %error,
                    "could not inspect a directory at the watch-scope depth \
                     limit; treating its children as unwatched",
                );
            }
        }
    }

    fn visit_file(&mut self, path: &Path, relative: &Path) {
        let relative_str = relative.to_string_lossy();
        if is_excluded_path(&relative_str) {
            return;
        }
        let name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".gitignore" || name == ".ignore" {
            self.scope.ignore_files.push(path.to_path_buf());
        }
    }

    fn finish(mut self, root: &Path, max_entries: usize, max_depth: usize) -> WatchScope {
        self.scope.ignore_files.sort();
        self.scope.watch_dirs.insert(root.to_path_buf());
        if self.entries_limited || self.depth_limited {
            self.scope.limit = Some(WatchScopeLimit {
                max_entries,
                max_depth,
                entries_seen: self.entries_seen,
                depth_limited: self.depth_limited,
            });
        }
        self.scope
    }
}

/// True when `path` has at least one directory entry. An entry that could
/// not even be read counts as present: a directory we can't inspect may
/// hold unwatched children, and the caller reports that as a partial scope.
fn directory_has_entries(path: &Path) -> std::io::Result<bool> {
    let mut entries = std::fs::read_dir(path)?;
    Ok(entries.next().is_some())
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
