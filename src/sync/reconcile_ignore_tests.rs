//! The incremental path must index exactly what a fresh publish would
//! index. These tests pin the watcher/full-publish inconsistency fix: a
//! gitignored dirty path (e.g. a `target/` build artifact after `cargo
//! build`) is skipped when the project's ignore rules are supplied, and
//! indexed when they are not.

use super::*;
use crate::sync::test_support::{setup_project, write};
use std::sync::Arc;

#[test]
fn gitignored_dirty_paths_are_not_indexed() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    write(project.path(), ".gitignore", b"target/\n");
    write(project.path(), "target/gen.rs", b"fn gen() {}\n");
    write(project.path(), "src/lib.rs", b"fn lib() {}\n");

    let scope = crate::watch::WatchScope::compute(project.path());
    let rules = crate::ignore_rules::IgnoreRules::build(project.path(), &scope.ignore_files);
    let dirty = ["target/gen.rs", "src/lib.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let deleted = HashSet::new();
    let limits = ResourceLimits::default();
    let options = ReconcileOptions {
        limits,
        deadline: Deadline::after(std::time::Duration::from_secs(60)),
        rules: Some(Arc::new(rules)),
    };

    let report = reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
        &options,
    )
    .expect("reconcile");

    assert_eq!(
        report.reconciled, 1,
        "only the indexable file is reconciled"
    );
    assert_eq!(
        report.ignored, 1,
        "the gitignored file is counted as skipped"
    );
    assert_eq!(report.deleted, 0);

    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM files WHERE path = 'target/gen.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        count, 0,
        "a gitignored file must not be indexed incrementally"
    );
    let lib_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM files WHERE path = 'src/lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(lib_count, 1, "the indexable file is stored");
}

/// Control for `gitignored_dirty_paths_are_not_indexed`: without the
/// rules, the same dirty set indexes the gitignored file — proving the
/// rules are what fix the inconsistency.
#[test]
fn without_rules_a_gitignored_dirty_path_would_be_indexed() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    write(project.path(), ".gitignore", b"target/\n");
    write(project.path(), "target/gen.rs", b"fn gen() {}\n");

    let dirty = ["target/gen.rs"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();
    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!(report.reconciled, 1);
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM files WHERE path = 'target/gen.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 1, "without rules the gitignored file gets indexed");
}
