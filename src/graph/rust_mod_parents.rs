//! Rust `#[path = "..."]` module-parent map and entry insertion.

use super::WorkspaceCrates;
use super::rust_path_attr::resolve_rust_path_attr_segments_in_context;
use std::collections::{BTreeMap, HashSet};
use std::hash::BuildHasher;
use std::path::Path;

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

pub(super) fn insert_rust_mod_parent_resolved(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_segments: Vec<String>,
    child_path: Option<String>,
) {
    let Some(child_path) = child_path else {
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

/// Inserts a `#[path = "..."]` module edge into `parents` using explicit
/// per-module `inline_segments` and context-aware `effective_rust_module_dir`
/// when the target path stays inside the project root.
pub(crate) fn insert_rust_mod_parent_segments_in_context<S: BuildHasher>(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_segments: Vec<String>,
    known_paths: &HashSet<&str, S>,
    workspace_crates: &WorkspaceCrates,
) {
    let child_path = resolve_rust_path_attr_segments_in_context(
        declaring_file,
        path_attr,
        &inline_segments,
        known_paths,
        workspace_crates,
    );
    insert_rust_mod_parent_resolved(
        parents,
        declaring_file,
        path_attr,
        inline_segments,
        child_path,
    );
}
