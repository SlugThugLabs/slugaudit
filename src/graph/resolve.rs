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
pub fn normalize_join(base_dir: &str, relative: &str) -> String {
    let mut segments: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// Returns the parent directory of a project-relative path.
pub fn parent_dir(relative_path: &str) -> String {
    Path::new(relative_path)
        .parent()
        .map(Path::to_string_lossy)
        .map_or_else(String::new, std::borrow::Cow::into_owned)
}

// Re-export resolver types for use by language-specific resolvers.

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
