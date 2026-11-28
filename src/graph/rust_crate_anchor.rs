//! Standalone compilation-target (`src/bin/*`, `tests/*`, `benches/*`,
//! `examples/*`, `build.rs`, and custom `[lib].path` / `[[bin]].path`)
//! anchoring for Rust `crate::` and `mod` resolution.

use super::WorkspaceCrates;
use super::cargo_workspace::PackageTargets;
use super::resolver::{KnownPaths, parent_dir};
use super::rust_mod_parent_walk::root_declaring_file;
use super::rust_mod_parents::RustModuleParents;
use super::rust_package_root::{
    file_under_dir, is_pkg_relative_target, join_pkg, split_named_target, split_package_root,
};

pub(super) struct CrateAnchor {
    pub(super) base_dir: String,
    pub(super) root_files: Vec<String>,
}

fn named_target_anchor(
    pkg_dir: &str,
    rel_in_pkg: &str,
    known_paths: &dyn KnownPaths,
) -> Option<CrateAnchor> {
    let (target_dir, after) = split_named_target(rel_in_pkg)?;
    match after.split_once('/') {
        // `<pkg>/<target_dir>/<name>.rs` -> crate root is that `.rs` file,
        // module base dir is `<pkg>/<target_dir>`.
        None => Some(CrateAnchor {
            base_dir: join_pkg(pkg_dir, target_dir),
            root_files: vec![join_pkg(pkg_dir, rel_in_pkg)],
        }),
        // `<pkg>/<target_dir>/<name>/main.rs` (or a submodule inside that
        // multi-file target when `.../main.rs` exists in `known_paths`).
        Some((seg, rest)) if !seg.is_empty() => {
            let main_root = join_pkg(pkg_dir, &format!("{target_dir}/{seg}/main.rs"));
            if rest == "main.rs" || known_paths.contains_path(&main_root) {
                return Some(CrateAnchor {
                    base_dir: join_pkg(pkg_dir, &format!("{target_dir}/{seg}")),
                    root_files: vec![main_root],
                });
            }
            // Shared helper directory under `<target_dir>/` (e.g.
            // `benches/common/mod.rs` with no `benches/common/main.rs`):
            // module base dir is `<pkg>/<target_dir>`.
            Some(CrateAnchor {
                base_dir: join_pkg(pkg_dir, target_dir),
                root_files: Vec::new(),
            })
        }
        Some(_) => None,
    }
}

fn target_submodule_anchor(
    pkg_dir: &str,
    rel_in_pkg: &str,
    anchor_file: &str,
    pkg: &PackageTargets,
    known_paths: &dyn KnownPaths,
    require_known: bool,
) -> Option<CrateAnchor> {
    let src_dir = join_pkg(pkg_dir, "src");
    let collect_matching = |only_known: bool| -> Vec<(&String, String)> {
        pkg.bin_paths
            .iter()
            .chain(pkg.extra_root_paths.iter())
            .filter(|target| !only_known || known_paths.contains_path(target.as_str()))
            .filter_map(|target| {
                let target_dir = parent_dir(target);
                if target_dir == src_dir || !file_under_dir(anchor_file, &target_dir) {
                    return None;
                }
                if target_dir == pkg_dir && is_pkg_relative_target(rel_in_pkg) {
                    return None;
                }
                Some((target, target_dir))
            })
            .collect()
    };
    let known = collect_matching(true);
    let candidates = if !known.is_empty() || require_known {
        known
    } else {
        collect_matching(false)
    };
    let best_len = candidates.iter().map(|(_, dir)| dir.len()).max()?;
    let base_dir = candidates
        .iter()
        .find_map(|(_, dir)| (dir.len() == best_len).then(|| dir.clone()))?;
    let root_files = candidates
        .into_iter()
        .filter(|(_, dir)| dir.len() == best_len)
        .map(|(target, _)| target.clone())
        .collect();
    Some(CrateAnchor {
        base_dir,
        root_files,
    })
}

fn lib_or_main_anchor(pkg_dir: &str, anchor_file: &str, custom_lib: Option<&str>) -> CrateAnchor {
    let src_dir = join_pkg(pkg_dir, "src");
    let main_candidate = join_pkg(pkg_dir, "src/main.rs");
    if anchor_file == main_candidate {
        return CrateAnchor {
            base_dir: src_dir,
            root_files: vec![main_candidate],
        };
    }
    let (base_dir, lib_root) = match custom_lib {
        Some(lib_path) => (parent_dir(lib_path), lib_path.to_owned()),
        None => (src_dir, join_pkg(pkg_dir, "src/lib.rs")),
    };
    let root_files = if anchor_file == lib_root || main_candidate == lib_root {
        vec![lib_root]
    } else {
        vec![lib_root, main_candidate]
    };
    CrateAnchor {
        base_dir,
        root_files,
    }
}

pub(super) fn crate_anchor(
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> CrateAnchor {
    let anchor_file = root_declaring_file(
        importing_relative_path,
        rust_parents,
        known_paths,
        workspace_crates,
    );
    let (pkg_dir, rel_in_pkg) = split_package_root(&anchor_file, known_paths, workspace_crates);

    // 1. `<pkg>/build.rs`: crate root is `<pkg>/build.rs`, base_dir is `<pkg>`.
    if rel_in_pkg == "build.rs" {
        return CrateAnchor {
            base_dir: pkg_dir.clone(),
            root_files: vec![join_pkg(&pkg_dir, "build.rs")],
        };
    }

    // 2. Explicit `[[bin]]`, `[[test]]`, `[[example]]`, or `[[bench]]` `path = "..."`
    //    in Cargo.toml when `anchor_file` is that target.
    let pkg = workspace_crates.package_at(&pkg_dir);
    if let Some(pkg) = pkg
        && pkg
            .bin_paths
            .iter()
            .chain(pkg.extra_root_paths.iter())
            .any(|target| target == &anchor_file)
    {
        return CrateAnchor {
            base_dir: parent_dir(&anchor_file),
            root_files: vec![anchor_file],
        };
    }

    // 3. `<pkg>/{src/bin,tests,benches,examples}/...` targets.
    let named_anchor = match named_target_anchor(&pkg_dir, rel_in_pkg, known_paths) {
        Some(anchor) if !anchor.root_files.is_empty() => return anchor,
        other => other,
    };

    // Submodules under `parent_dir(target)` when distinct from `<pkg>/src`.
    if let Some(pkg) = pkg
        && let Some(anchor) = target_submodule_anchor(
            &pkg_dir,
            rel_in_pkg,
            &anchor_file,
            pkg,
            known_paths,
            named_anchor.is_some(),
        )
    {
        return anchor;
    }

    if let Some(anchor) = named_anchor {
        return anchor;
    }

    // 4. Custom `[lib].path` from `WorkspaceCrates` (or default `src/{lib,main}.rs`).
    let custom_lib = pkg
        .or_else(|| {
            workspace_crates
                .enclosing_package(&anchor_file)
                .map(|(_, targets)| targets)
        })
        .and_then(|targets| targets.lib_path.as_deref());
    lib_or_main_anchor(&pkg_dir, &anchor_file, custom_lib)
}

#[cfg(test)]
#[path = "rust_crate_anchor_tests.rs"]
mod tests;
