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

use crate::ignore_rules::{indexable_walker, is_excluded_dir, is_excluded_path};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
}

impl WatchScope {
    /// Computes the scope by walking `root` with the same walker
    /// discovery uses. If the root can't be walked at all, the returned
    /// scope contains only the root itself.
    pub fn compute(root: &Path) -> WatchScope {
        let mut scope = WatchScope::default();
        for entry in indexable_walker(root) {
            let Ok(entry) = entry else { continue };
            let Some(relative) = entry.path().strip_prefix(root).ok() else {
                continue;
            };
            let relative_str = relative.to_string_lossy();
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if is_excluded_dir(&relative_str) {
                    continue;
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
        scope
    }
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
