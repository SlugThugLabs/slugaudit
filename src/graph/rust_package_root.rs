//! Cargo package root and non-stem `rustc` crate-root detection (`build.rs`,
//! `src/bin/*`, `tests/*`, `benches/*`, `examples/*`, and declared `Cargo.toml`
//! targets).

use super::WorkspaceCrates;
use super::resolver::{KnownPaths, parent_dir};

pub(super) const NAMED_TARGET_DIRS: [&str; 4] = ["src/bin", "tests", "benches", "examples"];

pub(super) fn strip_dir_prefix<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    path.strip_prefix(dir)
        .and_then(|rest| rest.strip_prefix('/'))
}

pub(super) fn is_pkg_relative_target(path: &str) -> bool {
    path == "build.rs"
        || ["src/", "tests/", "benches/", "examples/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub(super) fn join_pkg(pkg_dir: &str, sub: &str) -> String {
    if pkg_dir.is_empty() {
        sub.to_owned()
    } else if sub.is_empty() {
        pkg_dir.to_owned()
    } else {
        format!("{pkg_dir}/{sub}")
    }
}

pub(super) fn file_under_dir(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        !path.is_empty()
    } else {
        strip_dir_prefix(path, dir).is_some()
    }
}

pub(super) fn split_named_target(rel_in_pkg: &str) -> Option<(&'static str, &str)> {
    for target_dir in NAMED_TARGET_DIRS {
        if let Some(after) = strip_dir_prefix(rel_in_pkg, target_dir)
            && !after.is_empty()
        {
            return Some((target_dir, after));
        }
    }
    None
}

pub(super) fn infer_package_prefix(anchor_file: &str) -> Option<(String, &str)> {
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
            let rel = strip_dir_prefix(anchor_file, &dir).map_or(anchor_file, |rest| rest);
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
#[must_use]
pub(crate) fn is_non_stem_crate_root(
    relative_path: &str,
    known_paths: &dyn KnownPaths,
    workspace_crates: &WorkspaceCrates,
) -> bool {
    if workspace_crates.is_declared_crate_root(relative_path) {
        return true;
    }
    let (_, rel_in_pkg) = split_package_root(relative_path, known_paths, workspace_crates);
    rel_in_pkg == "build.rs"
        || split_named_target(rel_in_pkg).is_some_and(|(_, after)| !after.contains('/'))
}

/// Returns declared `[lib].path`, `[[bin]].path`, `[[test]].path`,
/// `[[example]].path`, and `[[bench]].path` crate-root files whose
/// `parent_dir` equals `dir`.
#[must_use]
pub(crate) fn crate_roots_in_dir<'a>(
    workspace_crates: &'a WorkspaceCrates,
    dir: &str,
) -> Vec<&'a str> {
    workspace_crates.crate_roots_in_dir(dir)
}
