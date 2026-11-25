//! Rust `#[path = "..."]` module-parent map and `super::` resolution walk.
//!
//! Kept apart from `resolve_rust` and `rust_paths` so each module owns one
//! clear job and stays well within the code-line limit.

use super::resolve::{normalize_join, parent_dir};
use super::resolver::{Resolution, ResolutionKind, unresolved};
use super::rust_paths::{
    first_resolved, has_glob_suffix, resolve_rust_from_base, rust_module_dir, strip_trailing_glob,
};
use std::collections::{BTreeMap, HashSet};
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

/// Joins each inline ancestor segment onto `start_dir` via `normalize_join`,
/// rejecting any absolute segment (starting with `/`) or any step that escapes
/// above the project root.
fn join_inline_segments<'a>(
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

/// Resolves a `#[path = "..."]` attribute from `declaring_file` (nested inside
/// `inline_dir`) to a normalized repo-relative child path, or `None` when the
/// path or `inline_dir` escapes the repo root or is absolute/empty.
#[cfg(test)]
#[must_use]
pub(crate) fn resolve_rust_path_attr(
    declaring_file: &str,
    path_attr: &str,
    inline_dir: &str,
) -> Option<String> {
    if inline_dir.is_empty() {
        return resolve_rust_path_attr_segments(declaring_file, path_attr, &[]);
    }
    resolve_rust_path_attr_segments(declaring_file, path_attr, &[inline_dir.to_owned()])
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
        // KEEP clone: implicit external mod under #[path] inline ancestor may be <name>.rs or <name>/mod.rs
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
pub(crate) fn insert_rust_mod_parent(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_dir: &str,
) {
    if inline_dir.starts_with('/') {
        return;
    }
    let segments = inline_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    insert_rust_mod_parent_segments(parents, declaring_file, path_attr, segments);
}

/// Resolves a Rust `super` or `super::...` reference, walking `rust_parents`
/// for `#[path = "..."]` declarations before falling back to module-tree
/// directory traversal.
pub(super) fn resolve_super_with_parents(
    text: &str,
    importing_relative_path: &str,
    known_paths: &HashSet<&str>,
    rust_parents: &RustModuleParents,
) -> Resolution {
    let Some((mut remaining_supers, rest)) = split_leading_supers(text) else {
        return unresolved();
    };
    let mut current_file = importing_relative_path;
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
            let Some(base) =
                join_inline_segments(&rust_module_dir(&parent.declaring_file), ancestors)
            else {
                return unresolved();
            };
            return resolve_under_base_or_declaring(parent, &base, rest, known_paths);
        }
        let Some(after) = remaining_supers.checked_sub(levels) else {
            break;
        };
        remaining_supers = after;
        current_file = parent.declaring_file.as_str();
    }

    let mut strict = rust_module_dir(current_file);
    let mut legacy = parent_dir(current_file);
    for _ in 0..remaining_supers {
        strict = parent_dir(&strict);
        legacy = parent_dir(&legacy);
    }
    let keep_legacy =
        !legacy.is_empty() && legacy != strict && strict != "src" && !strict.ends_with("/src");
    let mut bases = vec![strict];
    if keep_legacy {
        bases.push(legacy);
    }
    first_resolved(rest, &bases, known_paths)
}

fn split_leading_supers(text: &str) -> Option<(usize, &str)> {
    let mut count: usize = 1;
    let mut rest = text.split_once("::").map_or("", |(_, after)| after);
    while let Some(next) = rest.strip_prefix("super::") {
        rest = next;
        count = count.checked_add(1)?;
    }
    if rest == "super" {
        rest = "";
        count = count.checked_add(1)?;
    }
    Some((count, rest))
}

fn resolve_under_base_or_declaring(
    parent: &RustModuleParent,
    base: &str,
    rest: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    if !strip_trailing_glob(rest).is_empty() {
        let sub = resolve_rust_from_base(rest, base, known_paths, None);
        if sub.kind == ResolutionKind::Resolved {
            return sub;
        }
    }
    resolved_declaring_file(parent, rest, known_paths)
}

fn resolved_declaring_file(
    parent: &RustModuleParent,
    rest: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    if !known_paths.contains(parent.declaring_file.as_str()) {
        return unresolved();
    }
    let simple = rest.is_empty() || !has_glob_suffix(rest) || strip_trailing_glob(rest).is_empty();
    Resolution {
        kind: ResolutionKind::Resolved,
        confidence: Some(if simple { "High" } else { "Low" }),
        // KEEP clone: Resolution owns the resolved project-relative path String
        to_relative_path: Some(parent.declaring_file.clone()),
    }
}
