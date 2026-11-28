//! Cargo target-layout detection and Rust `super::` candidate base resolution.

use super::revision_rust_siblings::RUST_ROOT_FILES;
use super::revision_rust_targets::NAMED_TARGET_DIRS;
use crate::graph::resolver::{KnownPaths, normalize_join, parent_dir};
use crate::graph::{
    RustModuleParents, SuperParentWalk, WorkspaceCrates, crate_roots_in_dir,
    is_non_stem_crate_root, rust_module_dir, split_leading_supers, strip_trailing_glob,
    walk_super_parents,
};
use std::collections::HashSet;
use std::path::Path;

/// Graph scope context needed to trace Rust `super::` hops and crate roots
/// during incremental edge invalidation.
pub(super) struct RustScopeView<'a> {
    pub(super) known_paths: &'a dyn KnownPaths,
    pub(super) rust_parents: &'a RustModuleParents,
    pub(super) workspace_crates: &'a WorkspaceCrates,
}

/// Pre-indexed added and deleted Rust paths for constant-time directory and
/// `super::` lookups.
pub(super) struct ChangedRustPaths<'a> {
    pub(super) exact_paths: HashSet<&'a str>,
    pub(super) ancestor_dirs: HashSet<String>,
}

impl<'a> ChangedRustPaths<'a> {
    pub(super) fn new(added: &'a [String], deleted: &'a [String]) -> Self {
        // KEEP saturating: allocation capacity hint only
        let mut exact_paths = HashSet::with_capacity(added.len().saturating_add(deleted.len()));
        let mut ancestor_dirs = HashSet::new();
        for path in added.iter().chain(deleted.iter()) {
            if path.is_empty() {
                continue;
            }
            exact_paths.insert(path.as_str());
            let mut dir = parent_dir(path);
            loop {
                if dir.is_empty() {
                    ancestor_dirs.insert(dir);
                    break;
                }
                let next = parent_dir(&dir);
                if !ancestor_dirs.insert(dir) {
                    break;
                }
                dir = next;
            }
        }
        Self {
            exact_paths,
            ancestor_dirs,
        }
    }
}

pub(super) fn super_spec_targets_any_changed_path(
    file_path: &str,
    spec: &str,
    inline_segments: &[String],
    changed: &ChangedRustPaths<'_>,
    view: &RustScopeView<'_>,
) -> bool {
    if changed.exact_paths.is_empty() {
        return false;
    }
    let Some((supers, rest)) = split_leading_supers(spec) else {
        return false;
    };
    let first_part = strip_trailing_glob(rest)
        .split("::")
        .filter(|part| !part.is_empty())
        .map(strip_raw_prefix)
        .next();
    let bases = if supers <= inline_segments.len() {
        let Some(keep) = inline_segments.len().checked_sub(supers) else {
            return false;
        };
        inline_super_candidate_bases(file_path, inline_segments, keep, view)
    } else {
        let Some(remaining) = supers.checked_sub(inline_segments.len()) else {
            return false;
        };
        super_candidate_bases(file_path, remaining, view)
    };
    super_bases_can_match_path(&bases, first_part, changed, view.workspace_crates)
}

fn file_mod_dir(file_path: &str, view: &RustScopeView<'_>) -> String {
    if is_non_stem_crate_root(file_path, view.known_paths, view.workspace_crates) {
        parent_dir(file_path)
    } else {
        rust_module_dir(file_path)
    }
}

fn inline_super_candidate_bases(
    file_path: &str,
    inline_segments: &[String],
    keep: usize,
    view: &RustScopeView<'_>,
) -> Vec<String> {
    let Some(slice) = inline_segments.get(..keep) else {
        return Vec::new();
    };
    let mut dir = file_mod_dir(file_path, view);
    for seg in slice {
        if seg.starts_with('/') {
            return Vec::new();
        }
        if !seg.is_empty() {
            let Some(next) = normalize_join(&dir, seg) else {
                return Vec::new();
            };
            dir = next;
        }
    }
    vec![dir]
}

fn super_candidate_bases(file_path: &str, supers: usize, view: &RustScopeView<'_>) -> Vec<String> {
    if view.rust_parents.is_empty() {
        return module_tree_super_bases(file_path, supers, view);
    }
    match walk_super_parents(
        file_path,
        supers,
        view.rust_parents,
        view.known_paths,
        view.workspace_crates,
    ) {
        None => Vec::new(),
        Some(SuperParentWalk::InParent { base_dir, .. }) => vec![base_dir],
        Some(SuperParentWalk::Remaining { file, supers: rem }) => {
            module_tree_super_bases(&file, rem, view)
        }
    }
}

fn module_tree_super_bases(
    file_path: &str,
    supers: usize,
    view: &RustScopeView<'_>,
) -> Vec<String> {
    let mut strict = file_mod_dir(file_path, view);
    let mut legacy = parent_dir(file_path);
    for _ in 0..supers {
        strict = parent_dir(&strict);
        legacy = parent_dir(&legacy);
    }
    let keep_legacy =
        !legacy.is_empty() && legacy != strict && !is_multi_target_boundary(&strict, &legacy);
    let mut bases = vec![strict];
    if keep_legacy {
        bases.push(legacy);
    }
    bases
}

fn super_bases_can_match_path(
    bases: &[String],
    first_part: Option<&str>,
    changed: &ChangedRustPaths<'_>,
    workspace_crates: &WorkspaceCrates,
) -> bool {
    for base in bases {
        if (!base.is_empty()
            && (changed.exact_paths.contains(format!("{base}.rs").as_str())
                || RUST_ROOT_FILES.iter().any(|root| {
                    changed
                        .exact_paths
                        .contains(format!("{base}/{root}").as_str())
                })))
            || crate_roots_in_dir(workspace_crates, base)
                .iter()
                .any(|root| changed.exact_paths.contains(*root))
        {
            return true;
        }
        if let Some(first) = first_part
            && let Some(p0) = normalize_join(base, first)
            && !p0.is_empty()
            && (changed.exact_paths.contains(format!("{p0}.rs").as_str())
                || changed.ancestor_dirs.contains(p0.as_str()))
        {
            return true;
        }
    }
    false
}

pub(super) fn strip_raw_prefix(ident: &str) -> &str {
    match ident.strip_prefix("r#") {
        Some(stripped) => stripped,
        None => ident,
    }
}

fn is_multi_target_boundary(strict: &str, legacy: &str) -> bool {
    if Path::new(strict).ends_with("src") {
        return true;
    }
    let legacy_path = Path::new(legacy);
    NAMED_TARGET_DIRS
        .iter()
        .any(|dir| legacy_path.ends_with(dir))
}
