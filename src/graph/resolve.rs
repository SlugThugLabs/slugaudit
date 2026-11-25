//! Generic path-resolution utilities used by language-specific resolvers.
//!
//! These functions operate on project-relative forward-slash paths and are
//! language-agnostic — they don't know or care which language is being
//! resolved.

use std::path::Path;

/// Joins path components with `/` and normalizes `..`/`.` segments,
/// working on project-relative forward-slash paths rather than the host
/// OS's `Path` semantics (a project's stored paths are always `/`-joined,
/// see `sync::discovery`).
///
/// Returns `None` when a `..` would climb above the project root: such a
/// path names nothing inside the project, and dropping the excess `..`
/// would map `../x` from the root onto the in-project `x` (a false edge).
pub(super) fn normalize_join(base_dir: &str, relative: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for part in base_dir.split('/').chain(relative.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

/// Returns the parent directory of a project-relative path.
pub(super) fn parent_dir(relative_path: &str) -> String {
    match Path::new(relative_path).parent() {
        Some(parent) => parent.to_string_lossy().into_owned(),
        // A root or empty path has no parent directory.
        None => String::new(),
    }
}

// Re-export resolver types for use by language-specific resolvers.

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
