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

/// Resolves a `#[path = "..."]` attribute from `declaring_file` (nested inside
/// `inline_dir`) to a normalized repo-relative child path, or `None` when the
/// path escapes the repo root or is absolute/empty.
#[must_use]
pub(crate) fn resolve_rust_path_attr(
    declaring_file: &str,
    path_attr: &str,
    inline_dir: &str,
) -> Option<String> {
    if path_attr.is_empty() || path_attr.starts_with('/') {
        return None;
    }
    let declaring_dir = parent_dir(declaring_file);
    let base_dir = if inline_dir.is_empty() {
        declaring_dir
    } else {
        normalize_join(&declaring_dir, inline_dir)?
    };
    let joined = normalize_join(&base_dir, path_attr)?;
    (!joined.is_empty()).then_some(joined)
}

/// Inserts a `#[path = "..."]` module edge into `parents` when the target path
/// stays inside the project root.
pub(crate) fn insert_rust_mod_parent(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_dir: &str,
) {
    let Some(child_path) = resolve_rust_path_attr(declaring_file, path_attr, inline_dir) else {
        return;
    };
    let inline_ancestors = inline_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    parents.entry(child_path).or_insert(RustModuleParent {
        declaring_file: declaring_file.to_owned(),
        inline_ancestors,
    });
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
        let Some(next_hops) = hops.checked_add(1) else {
            break;
        };
        hops = next_hops;
        let inline_depth = parent.inline_ancestors.len();
        let Some(levels_in_declaring) = inline_depth.checked_add(1) else {
            break;
        };
        if remaining_supers < levels_in_declaring {
            return resolve_inline_parent(
                parent,
                inline_depth,
                remaining_supers,
                rest,
                known_paths,
            );
        }
        if remaining_supers == levels_in_declaring {
            return resolve_declaring_parent(parent, rest, known_paths);
        }
        let Some(remaining_after_declaring) = remaining_supers.checked_sub(levels_in_declaring)
        else {
            break;
        };
        remaining_supers = remaining_after_declaring;
        current_file = parent.declaring_file.as_str();
    }

    let mut strict = rust_module_dir(current_file);
    let mut legacy = parent_dir(current_file);
    for _ in 0..remaining_supers {
        strict = parent_dir(&strict);
        legacy = parent_dir(&legacy);
    }
    first_resolved(rest, &[strict, legacy], known_paths)
}

fn split_leading_supers(text: &str) -> Option<(usize, &str)> {
    let mut count: usize = 1;
    let mut rest = match text.split_once("::") {
        Some((_, after)) => after,
        None => "",
    };
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

fn resolve_inline_parent(
    parent: &RustModuleParent,
    inline_depth: usize,
    remaining_supers: usize,
    rest: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let Some(keep) = inline_depth
        .checked_add(1)
        .and_then(|total_levels| total_levels.checked_sub(remaining_supers))
    else {
        return unresolved();
    };
    if !strip_trailing_glob(rest).is_empty() {
        let mut inline_base = parent_dir(&parent.declaring_file);
        for segment in parent.inline_ancestors.iter().take(keep) {
            if !inline_base.is_empty() {
                inline_base.push('/');
            }
            inline_base.push_str(segment);
        }
        let sub = resolve_rust_from_base(rest, &inline_base, known_paths, None);
        if sub.kind == ResolutionKind::Resolved {
            return sub;
        }
    }
    resolved_declaring_file(parent, rest, known_paths)
}

fn resolve_declaring_parent(
    parent: &RustModuleParent,
    rest: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    if !strip_trailing_glob(rest).is_empty() {
        let base = rust_module_dir(&parent.declaring_file);
        let sub = resolve_rust_from_base(rest, &base, known_paths, None);
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
    let is_glob = has_glob_suffix(rest);
    let confidence = if rest.is_empty() || !is_glob || strip_trailing_glob(rest).is_empty() {
        Some("High")
    } else {
        Some("Low")
    };
    Resolution {
        kind: ResolutionKind::Resolved,
        confidence,
        // KEEP clone: Resolution owns the resolved project-relative path String
        to_relative_path: Some(parent.declaring_file.clone()),
    }
}
