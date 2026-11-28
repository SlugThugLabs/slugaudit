//! Rust `#[path = "..."]` attribute and inline `mod` segment path
//! normalization.

use super::WorkspaceCrates;
use super::resolver::{KnownPaths, normalize_join, parent_dir};
use super::rust_paths::effective_rust_module_dir;
#[cfg(test)]
use super::rust_paths::rust_module_dir;

/// Joins each inline ancestor segment onto `start_dir` via `normalize_join`,
/// rejecting any absolute segment (starting with `/`) or any step that escapes
/// above the project root.
pub(super) fn join_inline_segments<'a>(
    start_dir: &str,
    inline_segments: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut dir = start_dir.to_owned();
    for segment in inline_segments {
        if segment.starts_with('/') {
            return None;
        }
        if !segment.is_empty() {
            dir = normalize_join(&dir, segment)?;
        }
    }
    Some(dir)
}

fn resolve_rust_path_attr_inner(
    declaring_file: &str,
    path_attr: &str,
    inline_segments: &[String],
    mod_dir: impl FnOnce(&str) -> String,
) -> Option<String> {
    if path_attr.is_empty() || path_attr.starts_with('/') {
        return None;
    }
    let declaring_dir = if inline_segments.is_empty() {
        parent_dir(declaring_file)
    } else {
        mod_dir(declaring_file)
    };
    let base_dir =
        join_inline_segments(&declaring_dir, inline_segments.iter().map(String::as_str))?;
    let joined = normalize_join(&base_dir, path_attr)?;
    (!joined.is_empty()).then_some(joined)
}

/// Resolves a `#[path = "..."]` attribute from `declaring_file` (nested inside
/// `inline_segments`) to a normalized repo-relative child path, or `None` when
/// `path_attr` or any inline segment is absolute or escapes the repo root.
#[cfg(test)]
#[must_use]
pub(super) fn resolve_rust_path_attr_segments(
    declaring_file: &str,
    path_attr: &str,
    inline_segments: &[String],
) -> Option<String> {
    resolve_rust_path_attr_inner(declaring_file, path_attr, inline_segments, rust_module_dir)
}

/// Context-aware `#[path = "..."]` path resolver that uses
/// `effective_rust_module_dir` instead of `rust_module_dir` when
/// `inline_segments` is non-empty, handling non-stem crate roots such as
/// `build.rs`, `tests/*.rs`, and custom `[lib].path` / `[[bin]].path`.
#[must_use]
pub(super) fn resolve_rust_path_attr_segments_in_context(
    declaring_file: &str,
    path_attr: &str,
    inline_segments: &[String],
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Option<String> {
    resolve_rust_path_attr_inner(declaring_file, path_attr, inline_segments, |file| {
        effective_rust_module_dir(file, known_paths, workspace_crates)
    })
}

#[cfg(test)]
#[path = "rust_path_attr_tests.rs"]
mod tests;
