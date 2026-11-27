//! Cargo package root and standalone compilation-target (`src/bin/*`, `tests/*`,
//! `benches/*`, `examples/*`, `build.rs`, and custom `[lib].path` / `[[bin]].path`)
//! anchoring for Rust `crate::` and `mod` resolution.

use super::WorkspaceCrates;
use super::resolver::{KnownPaths, parent_dir};
use super::rust_mod_parents::{RustModuleParents, root_declaring_file};

const NAMED_TARGET_DIRS: [&str; 4] = ["src/bin", "tests", "benches", "examples"];

pub(super) struct CrateAnchor {
    pub(super) base_dir: String,
    pub(super) root_files: Vec<String>,
}

fn is_pkg_relative_target(path: &str) -> bool {
    path == "build.rs"
        || ["src/", "tests/", "benches/", "examples/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

fn join_pkg(pkg_dir: &str, sub: &str) -> String {
    if pkg_dir.is_empty() {
        sub.to_owned()
    } else if sub.is_empty() {
        pkg_dir.to_owned()
    } else {
        format!("{pkg_dir}/{sub}")
    }
}

fn infer_package_prefix(anchor_file: &str) -> Option<(String, &str)> {
    for (idx, ch) in anchor_file.char_indices() {
        if ch == '/'
            && let Some(next_idx) = idx.checked_add(1)
            && let (Some(prefix), Some(rest)) =
                (anchor_file.get(..idx), anchor_file.get(next_idx..))
            && !prefix.is_empty()
            && is_pkg_relative_target(rest)
        {
            return Some((prefix.to_owned(), rest));
        }
    }
    None
}

pub(super) fn split_package_root<'a>(
    anchor_file: &'a str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> (String, &'a str) {
    let mut dir = parent_dir(anchor_file);
    while !dir.is_empty() {
        if known_paths.contains_path(&format!("{dir}/Cargo.toml"))
            || workspace_crates.package_at(&dir).is_some()
        {
            let rel = anchor_file
                .strip_prefix(&dir)
                .and_then(|rest| rest.strip_prefix('/'))
                .map_or(anchor_file, |rest| rest);
            return (dir, rel);
        }
        dir = parent_dir(&dir);
    }
    if is_pkg_relative_target(anchor_file) || workspace_crates.package_at("").is_some() {
        return (String::new(), anchor_file);
    }
    match infer_package_prefix(anchor_file) {
        Some(pair) => pair,
        None => (String::new(), anchor_file),
    }
}

/// Returns `true` when `relative_path` is a `rustc` crate-root file whose
/// child `mod` declarations live in `parent_dir(relative_path)` rather than in
/// a `<stem>/` subdirectory (`build.rs`, `<target_dir>/<name>.rs`, or an
/// explicit `[lib].path` / `[[bin]].path` from `Cargo.toml`).
pub(super) fn is_non_stem_crate_root(
    relative_path: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> bool {
    if workspace_crates.is_declared_crate_root(relative_path) {
        return true;
    }
    let (_, rel_in_pkg) = split_package_root(relative_path, known_paths, workspace_crates);
    if rel_in_pkg == "build.rs" {
        return true;
    }
    NAMED_TARGET_DIRS.iter().any(|dir| {
        rel_in_pkg
            .strip_prefix(dir)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|after| !after.is_empty() && !after.contains('/'))
    })
}

fn named_target_anchor(
    pkg_dir: &str,
    rel_in_pkg: &str,
    known_paths: &dyn KnownPaths,
) -> Option<CrateAnchor> {
    for target_dir in NAMED_TARGET_DIRS {
        let Some(after) = rel_in_pkg
            .strip_prefix(target_dir)
            .and_then(|rest| rest.strip_prefix('/'))
        else {
            continue;
        };
        if after.is_empty() {
            continue;
        }
        match after.split_once('/') {
            // `<pkg>/<target_dir>/<name>.rs` -> crate root is that `.rs` file,
            // module base dir is `<pkg>/<target_dir>`.
            None => {
                return Some(CrateAnchor {
                    base_dir: join_pkg(pkg_dir, target_dir),
                    root_files: vec![join_pkg(pkg_dir, rel_in_pkg)],
                });
            }
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
                return Some(CrateAnchor {
                    base_dir: join_pkg(pkg_dir, target_dir),
                    root_files: Vec::new(),
                });
            }
            Some(_) => {}
        }
    }
    None
}

pub(super) fn crate_anchor(
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> CrateAnchor {
    let anchor_file = root_declaring_file(importing_relative_path, rust_parents);
    let (pkg_dir, rel_in_pkg) = split_package_root(anchor_file, known_paths, workspace_crates);

    // 1. `<pkg>/build.rs`: crate root is `<pkg>/build.rs`, base_dir is `<pkg>`.
    if rel_in_pkg == "build.rs" {
        return CrateAnchor {
            base_dir: pkg_dir.clone(),
            root_files: vec![join_pkg(&pkg_dir, "build.rs")],
        };
    }

    // 2. Explicit `[[bin]] path = "..."` in Cargo.toml when `anchor_file` is that bin.
    if let Some(pkg) = workspace_crates.package_at(&pkg_dir)
        && pkg.bin_paths.iter().any(|bin| bin == anchor_file)
    {
        return CrateAnchor {
            base_dir: parent_dir(anchor_file),
            root_files: vec![anchor_file.to_owned()],
        };
    }

    // 3. `<pkg>/{src/bin,tests,benches,examples}/...` targets.
    if let Some(anchor) = named_target_anchor(&pkg_dir, rel_in_pkg, known_paths) {
        return anchor;
    }

    // 4. Custom `[lib].path` from `WorkspaceCrates` (or package enclosing a
    // custom lib directory outside `src/`).
    if let Some(pkg) = workspace_crates.package_at(&pkg_dir).or_else(|| {
        workspace_crates
            .enclosing_package(anchor_file)
            .map(|(_, t)| t)
    }) && let Some(lib_path) = pkg.lib_path.as_deref()
    {
        let main_candidate = join_pkg(&pkg_dir, "src/main.rs");
        let mut root_files = vec![lib_path.to_owned()];
        if main_candidate != lib_path {
            root_files.push(main_candidate);
        }
        return CrateAnchor {
            base_dir: parent_dir(lib_path),
            root_files,
        };
    }

    CrateAnchor {
        base_dir: join_pkg(&pkg_dir, "src"),
        root_files: vec![
            join_pkg(&pkg_dir, "src/lib.rs"),
            join_pkg(&pkg_dir, "src/main.rs"),
        ],
    }
}

#[cfg(test)]
#[path = "rust_crate_anchor_tests.rs"]
mod tests;
