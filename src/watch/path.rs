//! Path normalization for the filesystem watcher.
//!
//! Kept separate from `state.rs` so the locking layer doesn't carry a
//! small string helper that's only used by the manager's event
//! dispatch. Lives next to the type definitions because both are pure,
//! non-locking helpers used by `manager.rs`.

use std::path::Path;

/// Normalizes a filesystem path to a project-relative, forward-slash path
/// string. Returns `None` if the path isn't under `root`.
///
/// Windows backslashes are normalized to forward slashes so the resulting
/// string matches what the rest of SlugAudit uses (path values stored in
/// the SQLite `files.path` column, the watcher's dirty/deleted sets, and
/// barrier-sync keys). Anything that inserts or removes a path in the
/// watcher goes through this — without it, a Windows project would have
/// `a\b\c.rs` in the watcher set but `a/b/c.rs` in the database.
pub fn normalize_relative_path(root: &Path, absolute: &Path) -> Option<String> {
    absolute
        .strip_prefix(root)
        .ok()?
        .to_str()
        .map(|s| s.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_relative_path_rejects_outside_root() {
        let root = Path::new("/project");
        let outside = Path::new("/elsewhere/file.rs");
        assert_eq!(normalize_relative_path(root, outside), None);
    }

    #[test]
    fn normalize_relative_path_strips_prefix() {
        let root = Path::new("/project");
        let inside = Path::new("/project/src/lib.rs");
        assert_eq!(
            normalize_relative_path(root, inside).as_deref(),
            Some("src/lib.rs"),
        );
    }

    #[test]
    fn normalize_relative_path_uses_forward_slashes() {
        // We can't easily synthesize a Windows path on a Unix test runner,
        // but the function's correctness is provable with manual escape
        // sequences the way callers actually pass them on Windows.
        let root = Path::new("/project");
        // Pretend a Windows caller passes `src\\nested\\file.rs` — we
        // verify the replacement happens unconditionally on backslashes.
        let inside = Path::new("src\\nested\\file.rs");
        // This path isn't under `/project`, so the function returns None —
        // but if the prefix did match, the replacement would be applied.
        assert_eq!(normalize_relative_path(root, inside), None);

        let root_match = Path::new("/project");
        let inside_match = Path::new("/project/src\\nested\\file.rs");
        assert_eq!(
            normalize_relative_path(root_match, inside_match).as_deref(),
            Some("src/nested/file.rs"),
        );
    }
}
