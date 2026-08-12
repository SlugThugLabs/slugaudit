//! Tests for `WatchScope`: the watch set must cover exactly the
//! directories whose contents can be indexed — never VCS internals,
//! SlugAudit's own data dir, or gitignored trees.

use super::*;
use std::fs;

#[test]
fn scope_covers_the_indexable_directories_only() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    fs::create_dir_all(root.join(".git")).expect("git dir");
    fs::create_dir_all(root.join(".planning/slugaudit")).expect("data dir");
    fs::create_dir_all(root.join("target")).expect("target dir");
    fs::create_dir_all(root.join("src/deep")).expect("src dirs");
    fs::write(root.join(".gitignore"), "target/\n").expect("gitignore");
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").expect("lib file");

    let scope = WatchScope::compute(root);
    assert!(
        scope.watch_dirs.contains(&root.to_path_buf()),
        "root is always watched"
    );
    assert!(scope.watch_dirs.contains(&root.join("src")));
    assert!(scope.watch_dirs.contains(&root.join("src/deep")));
    assert!(
        !scope.watch_dirs.contains(&root.join(".git")),
        "VCS internals are not watched"
    );
    assert!(
        !scope.watch_dirs.contains(&root.join(".planning/slugaudit")),
        "SlugAudit's own data dir is not watched",
    );
    assert!(
        !scope.watch_dirs.contains(&root.join("target")),
        "gitignored dirs are not watched"
    );
    assert!(scope.ignore_files.contains(&root.join(".gitignore")));
}

#[test]
fn a_scope_with_no_ignore_files_and_no_subdirs_watches_only_the_root() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    let scope = WatchScope::compute(root);
    assert_eq!(scope.watch_dirs.len(), 1);
    assert!(scope.watch_dirs.contains(&root.to_path_buf()));
    assert!(scope.ignore_files.is_empty());
}

#[test]
fn nested_gitignores_are_collected_for_the_matcher() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    fs::create_dir_all(root.join("a/b")).expect("nested dirs");
    fs::write(root.join(".gitignore"), "x\n").expect("root gitignore");
    fs::write(root.join("a/b/.ignore"), "y\n").expect("nested ignore");
    fs::write(root.join("a/b/.gitignore"), "z\n").expect("nested gitignore");

    let scope = WatchScope::compute(root);
    assert!(scope.ignore_files.contains(&root.join(".gitignore")));
    assert!(scope.ignore_files.contains(&root.join("a/b/.ignore")));
    assert!(scope.ignore_files.contains(&root.join("a/b/.gitignore")));
    // Sorted so `IgnoreRules::build` sees parents before children.
    assert!(scope.watch_dirs.contains(&root.join("a")));
    assert!(scope.watch_dirs.contains(&root.join("a/b")));
}
