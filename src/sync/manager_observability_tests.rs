//! Tests for `SourceSyncManager`'s observability surface: the
//! last-sync timestamp, watcher-state accessors, and the direct
//! `reconcile` entry point. Kept separate from `manager_tests.rs` so
//! each file stays under the source-size cap.

use crate::sync::SourceSyncManager;
use crate::sync::race_hook;
use crate::watch::WatcherHealth;
use std::fs;

fn create_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::create_dir_all(dir.path().join(".planning").join("slugaudit"))
        .expect("create activation dir");
    dir
}

fn write_file(project: &tempfile::TempDir, relative: &str, content: &[u8]) {
    let path = project.path().join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write file");
}

fn sync_project(manager: &SourceSyncManager, project: &tempfile::TempDir) {
    manager
        .ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
        .expect("sync succeeds");
}

fn open_db(project: &tempfile::TempDir) -> rusqlite::Connection {
    let database = project.path().join(".planning/slugaudit/project.db");
    crate::store::open_read_write(&database).expect("open db")
}

#[test]
fn last_sync_timestamp_is_zero_before_first_sync_and_stamped_after() {
    let manager = SourceSyncManager::new();
    assert_eq!(manager.last_sync_unix_seconds(), 0, "nothing synced yet");

    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    assert!(
        manager.last_sync_unix_seconds() > 0,
        "a successful ensure_current must stamp the timestamp"
    );
}

#[test]
fn watcher_state_accessors_move_from_empty_to_registered() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    assert!(manager.active_watch_state().is_none(), "no project yet");
    assert!(manager.watch_states_snapshot().is_empty());

    sync_project(&manager, &project);

    assert!(manager.active_watch_state().is_some(), "now registered");
    assert_eq!(manager.watch_states_snapshot().len(), 1);

    let root = project.path().canonicalize().expect("canonical root");
    let state = manager.watch_state_for(&root).expect("registered state");
    assert_eq!(
        state.health(),
        WatcherHealth::Unavailable,
        "a watcher-less manager reports Unavailable"
    );

    let other = tempfile::tempdir().expect("other dir");
    assert!(
        manager.watch_state_for(other.path()).is_none(),
        "an unknown root has no watch state"
    );
}

#[test]
fn reconcile_with_no_pending_events_is_a_noop() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    let state = manager.active_watch_state().expect("registered state");
    let mut connection = open_db(&project);
    manager
        .reconcile(project.path(), &state, &mut connection)
        .expect("reconcile with no events");

    let revision: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .expect("current revision");
    assert!(
        !revision.is_empty(),
        "the current revision survives a no-op reconcile"
    );
}

#[test]
fn reconcile_dirty_unchanged_paths_skips_revision_churn() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    let state = manager.active_watch_state().expect("registered state");
    state.mark_dirty("lib.rs".to_owned());
    let mut connection = open_db(&project);
    manager
        .reconcile(project.path(), &state, &mut connection)
        .expect("reconcile unchanged path");

    // The dirty path hashes to the stored hash, so no revision is
    // published.
    let count: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("revision count");
    assert_eq!(count, 1, "no new revision for an unchanged dirty path");
    assert_eq!(state.health(), WatcherHealth::Unavailable);
}

/// Arms a one-shot hook that corrupts the database mid-publish (after
/// sampling, before the diff/revision writes), so the publish fails with a
/// non-retryable database error. Deleting a source file instead would be
/// retryable (publish retries and indexes the deletion); corrupting the DB
/// cannot be retried into success. `ensure_current` must surface the
/// failure as an `ErrorData`, never panic and never leave a half-published
/// revision behind.
fn corrupt_database_hook(project: &tempfile::TempDir) {
    let project_path = project.path().to_path_buf();
    let hook_path = project_path.clone();
    race_hook::set(&project_path, move || {
        let database = hook_path.join(".planning/slugaudit/project.db");
        let _ = fs::write(database, b"corrupted mid-publish");
    });
}

#[test]
fn a_failing_publish_is_surfaced_as_an_error_not_a_panic() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    corrupt_database_hook(&project);
    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );

    assert!(
        result.is_err(),
        "a publish that fails mid-flight must surface as an error"
    );
}

/// Same guarantee for the watcher-backed verification path: a fresh
/// manager starts `NeedsVerification`, and a full-verification publish
/// that fails must surface as an error rather than a panic.
#[test]
fn a_failing_verification_publish_is_surfaced_as_an_error() {
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let manager1 = SourceSyncManager::with_watcher();
    sync_project(&manager1, &project);
    drop(manager1);

    let manager2 = SourceSyncManager::with_watcher();
    corrupt_database_hook(&project);
    let result = manager2.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );

    assert!(
        result.is_err(),
        "a failed verification publish must surface as an error"
    );
}
