//! Tests for the shared exclusions and the indexable walker: the hardcoded
//! exclusions, pruning at descent time, and the walker's project-scoped
//! ignore files.

use super::*;
use crate::sync::discover_and_hash;
use slugaudit_test_check::{Check, TestResult, ensure};
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

    let (files, _skipped) = discover_and_hash(&root).check("discover_and_hash")?;
    let indexed: std::collections::HashSet<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();

    ensure(
        indexed.contains("dist/out.rs"),
        format!("the walker must not apply a parent ignore file outside a git repo: {indexed:?}"),
    )?;
    Ok(())
}

#[test]
fn a_parent_gitignore_inside_a_git_repo_is_applied_to_a_nested_project() -> TestResult {
    let repo = project()?;
    fs::create_dir_all(repo.path().join(".git")).check("git dir")?;
    fs::write(repo.path().join(".gitignore"), "dist/\n").check("repo gitignore")?;
    let root = repo.path().join("nested-project");
    fs::create_dir_all(root.join("dist")).check("dist dir")?;
    fs::write(root.join("dist/out.rs"), "fn out() {}\n").check("dist file")?;
    fs::create_dir_all(root.join("src")).check("src dir")?;
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").check("lib file")?;

    let (files, _skipped) = discover_and_hash(&root).check("discover_and_hash")?;
    let indexed: std::collections::HashSet<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();

    ensure(
        !indexed.contains("dist/out.rs") && indexed.contains("src/lib.rs"),
        format!("the walker must apply an enclosing git repo's .gitignore: {indexed:?}"),
    )?;
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
    fs::write(root.join(".planning/PLAN.md"), "# plan").check("plan")?;
    fs::create_dir_all(root.join("target/debug")).check("target dir")?;
    fs::write(root.join("target/debug/out"), "x").check("target out")?;
    fs::create_dir_all(root.join("src")).check("src dir")?;
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").check("lib")?;

    let yielded: Vec<std::path::PathBuf> = indexable_walker(root)
        .map(|entry| entry.map(|entry| entry.path().to_path_buf()))
        .collect::<Result<_, _>>()
        .check("walk the project")?;

    ensure(
        yielded.iter().all(|path| {
            !path.starts_with(root.join(".git"))
                && !path.starts_with(root.join(".planning"))
                && !path.starts_with(root.join("target"))
        }),
        format!("excluded subtrees must not be yielded: {yielded:?}"),
    )?;
    ensure(
        yielded.iter().any(|path| path.ends_with("src/lib.rs")),
        "yielded.iter().any(|path| path.ends_with(\"src/lib.rs\"))",
    )?;
    Ok(())
}

#[test]
fn git_planning_and_target_directories_are_excluded() -> TestResult {
    for excluded in [
        ".planning/slugaudit",
        ".planning/slugaudit/project.db",
        ".planning/slugaudit/nested/deep.json",
        ".planning\\slugaudit\\project.db",
        ".planning",
        ".planning/PLAN.md",
        ".planning/README.md",
        "target",
        "target/debug/slugaudit",
        "crates/a/target/release/lib.rlib",
    ] {
        ensure(
            is_excluded_path(excluded),
            format!("{excluded} must be excluded"),
        )?;
        ensure(
            is_excluded_dir(excluded),
            format!("{excluded} must be excluded"),
        )?;
    }
    Ok(())
}

#[test]
fn non_excluded_siblings_and_lookalikes_stay_indexed() -> TestResult {
    for included in [
        "planning/notes.md",
        "src/target_utils.rs",
        "slugaudit/x.md",
        "src/lib.rs",
    ] {
        ensure(
            !is_excluded_path(included),
            format!("{included} must be indexed"),
        )?;
    }
    Ok(())
}
