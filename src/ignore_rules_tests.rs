//! Tests for the shared exclusions and the indexable walker: the hardcoded
//! exclusions, pruning at descent time, and the walker's project-scoped
//! ignore files.

use super::*;
use crate::sync::discover;
use slugaudit_test_check::{Check, TestResult};
use std::fs;

fn project() -> TestResult<tempfile::TempDir> {
    tempfile::tempdir().check("project dir")
}

/// The walker must not apply parent-directory ignore files above the
/// project root; project-scoped rules are the contract.
#[test]
fn a_parent_gitignore_is_not_applied_to_a_nested_project() -> TestResult {
    let repo = project()?;
    fs::write(repo.path().join(".gitignore"), "dist/\n").check("parent gitignore")?;
    let root = repo.path().join("nested-project");
    fs::create_dir_all(root.join("dist")).check("dist dir")?;
    fs::write(root.join("dist/out.rs"), "fn out() {}\n").check("dist file")?;
    fs::create_dir_all(root.join("src")).check("src dir")?;
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").check("lib file")?;

    let (files, _skipped) = discover(&root).check("discover")?;
    let indexed: std::collections::HashSet<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();

    assert!(
        indexed.contains("dist/out.rs"),
        "the walker must not apply a parent ignore file: {indexed:?}"
    );
    Ok(())
}

/// Excluded directories must be pruned at descent time. Filtering their
/// yielded files keeps them out of the index, but walking a large `.git` or
/// data directory still costs time on every sync.
#[test]
fn the_walker_does_not_descend_into_excluded_directories() -> TestResult {
    let project = project()?;
    let root = project.path();
    fs::create_dir_all(root.join(".git/objects")).check("git objects")?;
    for index in 0..32 {
        fs::write(root.join(format!(".git/objects/{index:02}")), "x").check("git object")?;
    }
    fs::create_dir_all(root.join(".planning/slugaudit")).check("data dir")?;
    for index in 0..32 {
        fs::write(root.join(format!(".planning/slugaudit/{index:02}.db")), "x").check("data")?;
    }
    fs::create_dir_all(root.join("src")).check("src dir")?;
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").check("lib")?;

    let yielded: Vec<std::path::PathBuf> = indexable_walker(root)
        .map(|entry| entry.map(|entry| entry.path().to_path_buf()))
        .collect::<Result<_, _>>()
        .check("walk the project")?;

    assert!(
        yielded
            .iter()
            .all(|path| !path.starts_with(root.join(".git"))
                && !path.starts_with(root.join(".planning/slugaudit"))),
        "excluded subtrees must not be yielded: {yielded:?}"
    );
    assert!(yielded.iter().any(|path| path.ends_with("src/lib.rs")));
    Ok(())
}

#[test]
fn the_runtime_data_directory_and_its_descendants_are_excluded() {
    for excluded in [
        ".planning/slugaudit",
        ".planning/slugaudit/project.db",
        ".planning/slugaudit/nested/deep.json",
        ".planning\\slugaudit\\project.db",
    ] {
        assert!(is_excluded_path(excluded), "{excluded} must be excluded");
        assert!(is_excluded_dir(excluded), "{excluded} must be excluded");
    }
}

#[test]
fn a_planning_sibling_sharing_the_runtime_dir_prefix_stays_indexed() {
    for included in [
        ".planning/slugaudit-notes/x.md",
        ".planning/slugaudit-notes",
        ".planning/slugauditx",
        ".planning/slugauditfoo/y.md",
        ".planning/other/z.md",
        ".planning/PLAN.md",
        ".planning",
        "slugaudit/x.md",
    ] {
        assert!(!is_excluded_path(included), "{included} must be indexed");
    }
}
