//! The watcher's pure decisions, kept apart from the lock and the `notify`
//! handle: which watches a scope change adds and removes, and what an event
//! on one project-relative path means for that project.

use super::scope::WatchScope;
use crate::ignore_rules::{IgnoreRules, is_excluded_path};
use std::path::{Path, PathBuf};

/// The directories to start and stop watching when `old` becomes `new`.
/// With no previous scope, the root is the only directory already
/// registered (by `watch_with_scope_limits`), so every other indexable
/// directory is new. Registration is non-recursive in both places, so this
/// difference is exactly the set of kernel watches.
pub(super) fn watch_dir_changes(
    old: Option<&WatchScope>,
    new: &WatchScope,
    root: &Path,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    match old {
        Some(old) => (
            new.watch_dirs
                .difference(&old.watch_dirs)
                .cloned()
                .collect(),
            old.watch_dirs
                .difference(&new.watch_dirs)
                .cloned()
                .collect(),
        ),
        None => (
            new.watch_dirs
                .iter()
                .filter(|dir| *dir != root)
                .cloned()
                .collect(),
            Vec::new(),
        ),
    }
}

/// What an event on one project-relative path means for its project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathClass {
    /// SlugAudit's own state or scratch files: never tracked.
    Excluded,
    /// A `.gitignore`/`.ignore`: tracked, and the rules it carries can change
    /// which directories are watched, so the scope must be recomputed on the
    /// next tool-thread pass before anything is reconciled.
    IgnoreFile,
    /// Ignored by the project's rules: never marked dirty. Reconcile applies
    /// the same filter, but a full publish skips these files, so the
    /// incremental path must too.
    Ignored,
    /// An indexable path: mark it dirty or deleted.
    Tracked,
}

pub(super) fn classify_path(relative: &str, rules: Option<&IgnoreRules>) -> PathClass {
    if is_excluded_path(relative) {
        PathClass::Excluded
    } else if is_ignore_file(relative) {
        PathClass::IgnoreFile
    } else if rules.is_some_and(|rules| rules.should_ignore(relative)) {
        PathClass::Ignored
    } else {
        PathClass::Tracked
    }
}

/// True if `relative` names a `.gitignore` or `.ignore` file at any depth.
fn is_ignore_file(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    name == ".gitignore" || name == ".ignore"
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
