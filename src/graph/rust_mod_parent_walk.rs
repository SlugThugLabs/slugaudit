// slugaudit-line-exception: approved-by=split-judge; reason=Single bounded parent-hop traversal concern across Rust #[path] edges and ordinary submodule ancestors with modular shared helpers and no internal duplication
//! Parent-hop traversal across Rust `#[path = "..."]` module-parent edges
//! and ordinary submodule ancestors.

use super::WorkspaceCrates;
use super::resolver::{KnownPaths, parent_dir};
use super::rust_mod_parents::{RustModuleParent, RustModuleParents};
use super::rust_package_root::is_non_stem_crate_root;
use super::rust_path_attr::join_inline_segments;
use super::rust_paths::effective_rust_module_dir;

/// Maximum `#[path]` parent hops before stopping to guard against cycles.
const MAX_RUST_PARENT_HOPS: usize = 32;

/// Outcome of walking `#[path]` parent hops for a `super::` chain.
pub(crate) enum SuperParentWalk {
    /// Stopped inside an inline ancestor or directly on `declaring_file`.
    InParent {
        /// Project-relative path of the declaring file.
        declaring_file: String,
        /// Effective module directory where remaining segments resolve.
        base_dir: String,
    },
    /// Exhausted `#[path]` entries (or hit the hop cap) with `supers` hops left.
    Remaining {
        /// File reached after exhausting `#[path]` parent hops.
        file: String,
        /// Number of `super::` hops remaining from `file`.
        supers: usize,
    },
}

enum ParentHop<'a> {
    PathAttr(&'a RustModuleParent),
    Ordinary(String),
}

fn ordinary_parent_module_file(
    file_path: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Option<String> {
    if is_non_stem_crate_root(file_path, known_paths, workspace_crates) {
        return None;
    }
    let dir = effective_rust_module_dir(file_path, known_paths, workspace_crates);
    let parent_mod_dir = parent_dir(&dir);
    if dir.is_empty() || parent_mod_dir.is_empty() || parent_mod_dir == dir {
        return None;
    }
    [
        format!("{parent_mod_dir}.rs"),
        format!("{parent_mod_dir}/mod.rs"),
    ]
    .into_iter()
    .find(|c| c != file_path && known_paths.contains_path(c))
}

fn next_ordinary_reaches_parent(
    file_path: &str,
    rust_parents: &RustModuleParents,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
    max_hops: usize,
) -> Option<String> {
    if rust_parents.is_empty() || max_hops == 0 {
        return None;
    }
    let first = ordinary_parent_module_file(file_path, known_paths, workspace_crates)?;
    let mut probe = first.clone();
    for _ in 0..max_hops {
        if rust_parents.contains_key(probe.as_str()) {
            return Some(first);
        }
        probe = ordinary_parent_module_file(&probe, known_paths, workspace_crates)?;
    }
    None
}

fn next_parent_step<'a>(
    current_file: &str,
    hops: &mut usize,
    rust_parents: &'a RustModuleParents,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Option<ParentHop<'a>> {
    if rust_parents.is_empty() || *hops >= MAX_RUST_PARENT_HOPS {
        return None;
    }
    let hop = if let Some(parent) = rust_parents.get(current_file) {
        ParentHop::PathAttr(parent)
    } else {
        let remaining_hops = MAX_RUST_PARENT_HOPS.checked_sub(*hops)?;
        let ordinary_parent = next_ordinary_reaches_parent(
            current_file,
            rust_parents,
            known_paths,
            workspace_crates,
            remaining_hops,
        )?;
        ParentHop::Ordinary(ordinary_parent)
    };
    *hops = hops.checked_add(1)?;
    Some(hop)
}

fn walk_parent_hops(
    start_file: &str,
    rust_parents: &RustModuleParents,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
    mut matches_host: impl FnMut(&str) -> bool,
) -> (String, bool) {
    let mut current = start_file.to_owned();
    let mut hops: usize = 0;
    while let Some(step) = next_parent_step(
        &current,
        &mut hops,
        rust_parents,
        known_paths,
        workspace_crates,
    ) {
        let next = match step {
            ParentHop::PathAttr(parent) => parent.declaring_file.clone(),
            ParentHop::Ordinary(ordinary_parent) => ordinary_parent,
        };
        if matches_host(&next) {
            return (next, true);
        }
        current = next;
    }
    (current, false)
}

/// Walks `rust_parents` (and ordinary submodule ancestors reaching a `#[path]`
/// entry) up to `MAX_RUST_PARENT_HOPS` to find the top-most declaring file
/// owning `start_file`.
#[must_use]
pub(crate) fn root_declaring_file(
    start_file: &str,
    rust_parents: &RustModuleParents,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> String {
    walk_parent_hops(
        start_file,
        rust_parents,
        known_paths,
        workspace_crates,
        |_| false,
    )
    .0
}

/// Returns `true` when `start_file` reaches a declaring host matching
/// `matches_host` along `rust_parents` and ordinary submodule ancestors.
#[must_use]
pub(crate) fn reaches_declaring_host(
    start_file: &str,
    rust_parents: &RustModuleParents,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
    matches_host: impl Fn(&str) -> bool,
) -> bool {
    walk_parent_hops(
        start_file,
        rust_parents,
        known_paths,
        workspace_crates,
        matches_host,
    )
    .1
}

/// Walks `#[path]` parent hops (and ordinary submodule ancestors reaching a
/// `#[path]` entry) for `initial_supers` leading `super::` segments.
/// Returns `None` if an inline ancestor path escapes the project root.
#[must_use]
pub(crate) fn walk_super_parents(
    start_file: &str,
    initial_supers: usize,
    rust_parents: &RustModuleParents,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> Option<SuperParentWalk> {
    let mut current_file = start_file.to_owned();
    let mut remaining_supers = initial_supers;
    let mut hops: usize = 0;

    while remaining_supers > 0 {
        let Some(step) = next_parent_step(
            &current_file,
            &mut hops,
            rust_parents,
            known_paths,
            workspace_crates,
        ) else {
            break;
        };
        let (declaring_file, inline_ancestors): (&str, &[String]) = match &step {
            ParentHop::PathAttr(parent) => (&parent.declaring_file, &parent.inline_ancestors),
            ParentHop::Ordinary(ordinary_parent) => (ordinary_parent.as_str(), &[]),
        };
        let Some(levels) = inline_ancestors.len().checked_add(1) else {
            break;
        };
        if let Some(keep) = levels.checked_sub(remaining_supers) {
            let parent_mod_dir =
                effective_rust_module_dir(declaring_file, known_paths, workspace_crates);
            let base_dir = if keep == 0 {
                parent_mod_dir
            } else {
                let ancestors = inline_ancestors.iter().take(keep).map(String::as_str);
                join_inline_segments(&parent_mod_dir, ancestors)?
            };
            return Some(SuperParentWalk::InParent {
                declaring_file: declaring_file.to_owned(),
                base_dir,
            });
        }
        let Some(after) = remaining_supers.checked_sub(levels) else {
            break;
        };
        remaining_supers = after;
        current_file = declaring_file.to_owned();
    }
    Some(SuperParentWalk::Remaining {
        file: current_file,
        supers: remaining_supers,
    })
}
