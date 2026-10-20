//! Tests for `WatchScope`: the watch set must cover exactly the
//! directories whose contents can be indexed — never VCS internals,
//! SlugAudit's own data dir, or gitignored trees.

use super::*;
use crate::store::test_capture::capture_warns;
use std::collections::{HashMap, HashSet};
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

#[test]
fn a_count_capped_scope_walk_is_reported_and_keeps_the_root() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    for name in ["a", "b", "c", "d", "e"] {
        fs::create_dir_all(root.join(name)).expect("dir");
    }

    let scope = WatchScope::compute_with_limits(root, 3, 64);

    let limit = scope
        .limit
        .expect("more entries than the cap must report a limit");
    assert_eq!(limit.max_entries, 3);
    assert_eq!(limit.entries_seen, 3, "at most `max_entries` are processed");
    assert!(!limit.depth_limited);
    assert!(
        scope.watch_dirs.contains(&root.to_path_buf()),
        "root is always watched"
    );
    assert!(
        scope.watch_dirs.len() <= 3,
        "the watch set cannot contain more directories than entries processed: {:?}",
        scope.watch_dirs
    );
}

#[test]
fn a_depth_capped_scope_walk_stops_at_the_limit_and_flags_it() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    fs::create_dir_all(root.join("a/b/c/d")).expect("nested dirs");

    let scope = WatchScope::compute_with_limits(root, 1_000, 2);

    let limit = scope
        .limit
        .expect("a directory at the depth cap with children must report a limit");
    assert_eq!(limit.max_depth, 2);
    assert!(
        limit.depth_limited,
        "a/b has an unwalked child at the depth cap"
    );
    assert!(scope.watch_dirs.contains(&root.to_path_buf()));
    assert!(scope.watch_dirs.contains(&root.join("a")));
    assert!(scope.watch_dirs.contains(&root.join("a/b")));
    assert!(
        !scope.watch_dirs.contains(&root.join("a/b/c")),
        "the walker must not descend past max_depth"
    );
}

/// R12: an `ignore::Error` from the walker carries the directory path in its
/// `Display` (`WithPath` renders `{path}: {err}`); the scope warning logs the
/// redacted path id and the category instead.
#[test]
fn a_scope_walk_error_logs_no_raw_path() {
    let project = tempfile::tempdir().expect("project dir");
    let missing = project.path().join("missing-subtree");
    let raw_path = missing.to_string_lossy().to_string();

    let (scope, logs) = capture_warns(|| {
        tracing::warn!("capture liveness control for watch_scope_walk");
        WatchScope::compute_with_limits(&missing, 100, 8)
    });

    assert!(
        scope.watch_dirs.contains(&missing),
        "the root is always watched even when the walk fails"
    );
    assert!(
        logs.contains("capture liveness control for watch_scope_walk"),
        "the capture must see this thread's events, captured: {logs}"
    );
    assert!(
        logs.contains("operation=\"watch_scope_walk\""),
        "the failure must name the operation, captured: {logs}"
    );
    assert!(
        logs.contains("error_kind=\"io\""),
        "the failure must carry the walker error category, captured: {logs}"
    );
    assert!(
        logs.contains("path_id="),
        "the failure must carry a redacted path id, captured: {logs}"
    );
    assert!(
        !logs.contains(&raw_path),
        "the raw directory path must not be logged, captured: {logs}"
    );
}

/// R12: a failed scope watch pairs the redacted directory id with a
/// path-free category. The notify backend attaches the raw path to the
/// error, which `Display` would have written into the log.
#[test]
fn a_failed_scope_watch_logs_no_raw_path() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path().to_path_buf();
    let missing = root.join("missing-directory");
    let raw_path = missing.to_string_lossy().to_string();
    let watcher = notify::RecommendedWatcher::new(
        |_event: notify::Result<notify::Event>| {},
        notify::Config::default(),
    )
    .expect("watcher");
    let mut inner = WatchManagerInner {
        projects: HashMap::from([(root.clone(), ProjectWatchInfo::default())]),
        watcher: Some(watcher),
    };
    let mut scope = WatchScope::default();
    scope.watch_dirs.insert(missing.clone());
    let rules = Arc::new(IgnoreRules::build(&root, &[]));

    let ((), logs) = capture_warns(|| {
        tracing::warn!("capture liveness control for scope_watch");
        apply_scope_locked(&mut inner, &root, scope, rules, 0);
    });

    assert!(
        logs.contains("capture liveness control for scope_watch"),
        "the capture must see this thread's events, captured: {logs}"
    );
    assert!(
        logs.contains("operation=scope_watch"),
        "the failure must name the operation, captured: {logs}"
    );
    assert!(
        logs.contains("error_kind=\"path_not_found\""),
        "the failure must carry the notify error category, captured: {logs}"
    );
    assert!(
        !logs.contains(&raw_path),
        "the raw directory path must not be logged, captured: {logs}"
    );
}

/// R12: a failed scope unwatch is logged the same way. Linux-only because
/// the inotify backend reports an unknown path as `WatchNotFound` while the
/// FSEvents backend treats removing an unknown path as a no-op.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_scope_unwatch_logs_no_raw_path() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path().to_path_buf();
    let never_watched = root.join("removed-directory");
    let raw_path = never_watched.to_string_lossy().to_string();
    let watcher = notify::RecommendedWatcher::new(
        |_event: notify::Result<notify::Event>| {},
        notify::Config::default(),
    )
    .expect("watcher");
    let info = ProjectWatchInfo {
        scope: Some(WatchScope {
            watch_dirs: HashSet::from([root.clone(), never_watched.clone()]),
            ..WatchScope::default()
        }),
        ..ProjectWatchInfo::default()
    };
    let mut inner = WatchManagerInner {
        projects: HashMap::from([(root.clone(), info)]),
        watcher: Some(watcher),
    };
    let mut scope = WatchScope::default();
    scope.watch_dirs.insert(root.clone());
    let rules = Arc::new(IgnoreRules::build(&root, &[]));

    let ((), logs) = capture_warns(|| {
        tracing::warn!("capture liveness control for scope_unwatch");
        apply_scope_locked(&mut inner, &root, scope, rules, 0);
    });

    assert!(
        logs.contains("capture liveness control for scope_unwatch"),
        "the capture must see this thread's events, captured: {logs}"
    );
    assert!(
        logs.contains("operation=scope_unwatch"),
        "the failure must name the operation, captured: {logs}"
    );
    assert!(
        logs.contains("error_kind=\"watch_not_found\""),
        "the failure must carry the notify error category, captured: {logs}"
    );
    assert!(
        !logs.contains(&raw_path),
        "the raw directory path must not be logged, captured: {logs}"
    );
}

#[test]
fn a_scope_exactly_at_the_entry_cap_is_not_reported_as_truncated() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").expect("lib file");

    // The walker yields exactly three entries: root, src, src/lib.rs.
    let scope = WatchScope::compute_with_limits(root, 3, 64);

    assert!(
        scope.limit.is_none(),
        "a tree exactly at the cap is complete, not truncated: {:?}",
        scope.limit
    );
}
