//! Fail-closed coverage for a sweep whose walk could not verify the tree.
//!
//! A walker error on a directory (or the project root) previously became
//! `Walked::Skipped`, so `stored - on_disk` published every row under that
//! subtree as deleted while reporting the watcher `Healthy`. These tests
//! pin the replacement contract: the sweep returns a typed failure, no
//! deletion is published, and the failure log carries only a static kind
//! and a redacted path id.

use super::*;
use crate::model::ResourceLimits;
use crate::sync::test_support::{setup_project, stored_paths, write};
use crate::util::Deadline;

fn generous_deadline() -> Deadline {
    Deadline::after(ResourceLimits::default().max_sync_wall_clock)
}

/// A path the walker will fail to list even when the process runs as root:
/// a root that does not exist. The old code turned its single walker error
/// into a skip and published every stored row as deleted.
#[test]
fn a_missing_project_root_fails_the_sweep_and_deletes_no_stored_row() {
    let (project, _db_dir, connection, revision) = setup_project();
    let missing_root = project.path().join("gone");

    let error = match sweep(&connection, &missing_root, &generous_deadline()) {
        Ok(_) => panic!("an unlistable root must fail the sweep"),
        Err(error) => error,
    };

    assert!(
        matches!(&error, SweepError::IncompleteWalk { kind } if *kind == "io"),
        "expected a typed incomplete walk, got {error:?}"
    );
    assert_eq!(
        stored_paths(&connection),
        ["a.rs", "b.rs"],
        "a failed walk must not be treated as a deletion set"
    );
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .expect("read the current revision");
    assert_eq!(
        current, revision,
        "a failed walk must not publish a revision"
    );
}

/// A malformed ignore file is attached to a walked `DirEntry` and its rules
/// are dropped. The sweep must fail closed instead of treating the files it
/// would have excluded as part of `on_disk`.
#[test]
fn an_unapplied_ignore_file_fails_the_sweep_instead_of_dropping_its_rules() {
    let (project, _db_dir, connection, revision) = setup_project();
    write(project.path(), ".gitignore", b"[z-a]\n");

    let error = match sweep(&connection, project.path(), &generous_deadline()) {
        Ok(_) => panic!("an unapplied ignore file must fail the sweep"),
        Err(error) => error,
    };

    assert!(
        matches!(&error, SweepError::IncompleteWalk { kind } if *kind == "glob"),
        "expected the walker's glob category, got {error:?}"
    );
    assert_eq!(stored_paths(&connection), ["a.rs", "b.rs"]);
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .expect("read the current revision");
    assert_eq!(current, revision);
}

/// The diagnostic for a directory walk error carries the static
/// `ignore::Error` category, the real path and the error's `Display` as the
/// cause. It is
/// emitted at debug level; the failure contract's operation line is the
/// single warn.
#[test]
fn honest_log_a_walk_error_is_logged_with_its_kind_real_path_and_cause() {
    let root = std::path::PathBuf::from("/home/private-client/checkout");
    let secret_dir = root.join("secret-dir");
    let error = ignore::Error::WithPath {
        path: secret_dir.clone(),
        err: Box::new(ignore::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ))),
    };

    let (result, logs) = crate::store::test_capture::capture_at_level(
        || {
            tracing::warn!("control-line-sweep-walk-probe");
            walked_file(&root, Err(error), &mut HashSet::new())
        },
        tracing::Level::DEBUG,
    );

    let kind = match result {
        Ok(_) => panic!("a walk error must fail closed"),
        Err(kind) => kind,
    };
    assert_eq!(kind, "io", "the typed failure carries the static category");
    assert!(
        logs.contains("control-line-sweep-walk-probe"),
        "the capture must be live: {logs}"
    );
    let walk_lines: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("operation=\"stat_sweep_walk\""))
        .collect();
    assert_eq!(
        walk_lines.len(),
        1,
        "the walk failure must be logged exactly once: {logs}"
    );
    assert!(
        walk_lines[0].contains("error_kind=\"io\""),
        "the log must carry the static category: {}",
        walk_lines[0]
    );
    assert!(
        walk_lines[0].contains("path=Some(\"/home/private-client/checkout/secret-dir\")"),
        "the log must carry the real path: {}",
        walk_lines[0]
    );
    assert!(
        walk_lines[0].contains("permission denied"),
        "the log must carry the underlying cause: {}",
        walk_lines[0]
    );
}

/// The full failure contract: a walk failure surfaces as an internal error
/// whose message is path-free, marks the watcher `Desynced`, and leaves the
/// stored file set untouched.
#[test]
fn a_failed_walk_marks_the_watcher_desynced_and_keeps_the_stored_rows() {
    use crate::progress::NoopProgressSink;
    use crate::watch::{WatchState, WatcherHealth};

    let (project, _db_dir, connection, _revision) = setup_project();
    let error = match crate::sync::sweep_reconcile::sweep_tree(
        &project.path().join("gone"),
        &connection,
        &generous_deadline(),
    ) {
        Ok(_) => panic!("an unlistable root must fail the sweep"),
        Err(error) => error,
    };
    assert!(
        error.message.contains("could not verify the tree")
            && error.message.contains("no rows were deleted"),
        "the failure must say the tree was not verified: {error:?}"
    );

    let state = WatchState::new();
    let error = crate::sync::sweep_reconcile::sweep_failed(
        project.path(),
        &state,
        &NoopProgressSink,
        error,
    );
    assert_eq!(
        state.health(),
        WatcherHealth::Desynced,
        "an unverified tree must not leave the watcher Healthy"
    );
    assert!(
        error.data.is_none(),
        "a walk failure is not a coded budget failure"
    );
    assert_eq!(stored_paths(&connection), ["a.rs", "b.rs"]);
}
