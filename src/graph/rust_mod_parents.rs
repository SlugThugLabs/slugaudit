//! Rust `#[path = "..."]` module-parent map, path-attribute normalization,
//! and parent-hop walking.

use super::resolver::{normalize_join, parent_dir};
use super::rust_paths::rust_module_dir;
use std::collections::BTreeMap;
use std::path::Path;

/// Maximum `#[path]` parent hops before stopping to guard against cycles.
const MAX_RUST_PARENT_HOPS: usize = 32;

/// Declaring file and inline module nesting for a Rust file included via
/// `#[path = "..."] mod name;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RustModuleParent {
    /// Project-relative path of the file containing the `#[path]` `mod` item.
    pub(crate) declaring_file: String,
    /// Enclosing inline `mod` names between `declaring_file` and the `#[path]`
    /// `mod` item (`inline_dir` split on `/`).
    pub(crate) inline_ancestors: Vec<String>,
}

/// Map from `#[path]` target file path to its declaring parent module info.
pub(crate) type RustModuleParents = BTreeMap<String, RustModuleParent>;

/// Outcome of walking `#[path]` parent hops for a `super::` chain.
pub(super) enum SuperParentWalk<'a> {
    /// Stopped inside an inline ancestor or directly on `declaring_file`.
    InParent {
        declaring_file: &'a str,
        base_dir: String,
    },
    /// Exhausted `#[path]` entries (or hit the hop cap) with `supers` hops left.
    Remaining { file: &'a str, supers: usize },
}

/// Splits a `/`-joined `inline_dir` string into non-empty inline ancestor
/// segment names.
#[cfg(test)]
#[must_use]
pub(crate) fn split_inline_dir(inline_dir: &str) -> Vec<String> {
    inline_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Walks `rust_parents` up to [`MAX_RUST_PARENT_HOPS`] to find the top-most
/// declaring file owning `start_file`.
pub(super) fn root_declaring_file<'a>(
    start_file: &'a str,
    rust_parents: &'a RustModuleParents,
) -> &'a str {
    let mut current = start_file;
    for _ in 0..MAX_RUST_PARENT_HOPS {
        let Some(parent) = rust_parents.get(current) else {
            break;
        };
        current = parent.declaring_file.as_str();
    }
    current
}

/// Walks `#[path]` parent hops for `initial_supers` leading `super::` segments.
/// Returns `None` if an inline ancestor path escapes the project root.
pub(super) fn walk_super_parents<'a>(
    start_file: &'a str,
    initial_supers: usize,
    rust_parents: &'a RustModuleParents,
) -> Option<SuperParentWalk<'a>> {
    let mut current_file = start_file;
    let mut remaining_supers = initial_supers;
    let mut hops: usize = 0;

    while remaining_supers > 0 && hops < MAX_RUST_PARENT_HOPS {
        let Some(parent) = rust_parents.get(current_file) else {
            break;
        };
        let (Some(next_hops), Some(levels)) = (
            hops.checked_add(1),
            parent.inline_ancestors.len().checked_add(1),
        ) else {
            break;
        };
        hops = next_hops;
        if let Some(keep) = levels.checked_sub(remaining_supers) {
            let ancestors = parent
                .inline_ancestors
                .iter()
                .take(keep)
                .map(String::as_str);
            let base_dir =
                join_inline_segments(&rust_module_dir(&parent.declaring_file), ancestors)?;
            return Some(SuperParentWalk::InParent {
                declaring_file: parent.declaring_file.as_str(),
                base_dir,
            });
        }
        let Some(after) = remaining_supers.checked_sub(levels) else {
            break;
        };
        remaining_supers = after;
        current_file = parent.declaring_file.as_str();
    }
    Some(SuperParentWalk::Remaining {
        file: current_file,
        supers: remaining_supers,
    })
}

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

/// Resolves a `#[path = "..."]` attribute from `declaring_file` (nested inside
/// `inline_segments`) to a normalized repo-relative child path, or `None` when
/// `path_attr` or any inline segment is absolute or escapes the repo root.
#[must_use]
pub(crate) fn resolve_rust_path_attr_segments(
    declaring_file: &str,
    path_attr: &str,
    inline_segments: &[String],
) -> Option<String> {
    if path_attr.is_empty() || path_attr.starts_with('/') {
        return None;
    }
    let declaring_dir = if inline_segments.is_empty() {
        parent_dir(declaring_file)
    } else {
        rust_module_dir(declaring_file)
    };
    let base_dir =
        join_inline_segments(&declaring_dir, inline_segments.iter().map(String::as_str))?;
    let joined = normalize_join(&base_dir, path_attr)?;
    (!joined.is_empty()).then_some(joined)
}

/// Inserts a `#[path = "..."]` module edge into `parents` using explicit
/// per-module `inline_segments` when the target path stays inside the project
/// root.
pub(crate) fn insert_rust_mod_parent_segments(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_segments: Vec<String>,
) {
    let Some(child_path) =
        resolve_rust_path_attr_segments(declaring_file, path_attr, &inline_segments)
    else {
        return;
    };
    let parent = RustModuleParent {
        declaring_file: declaring_file.to_owned(),
        inline_ancestors: inline_segments,
    };
    if Path::new(path_attr).extension().is_some() {
        parents.entry(child_path).or_insert(parent);
    } else {
        parents
            .entry(format!("{child_path}.rs"))
            .or_insert_with(|| parent.clone());
        parents
            .entry(format!("{child_path}/mod.rs"))
            .or_insert(parent);
    }
}

/// Inserts a `#[path = "..."]` module edge into `parents` when the target path
/// stays inside the project root.
#[cfg(test)]
pub(crate) fn insert_rust_mod_parent(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_dir: &str,
) {
    if inline_dir.starts_with('/') {
        return;
    }
    insert_rust_mod_parent_segments(
        parents,
        declaring_file,
        path_attr,
        split_inline_dir(inline_dir),
    );
}
