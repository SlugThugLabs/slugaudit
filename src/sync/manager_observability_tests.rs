//! Tests for `SourceSyncManager`'s observability surface: the
//! last-sync timestamp, watcher-state accessors, and the direct
//! `reconcile` entry point. Kept separate from `manager_tests.rs` so
//! each file stays under the source-size cap.

use crate::sync::SourceSyncManager;
use crate::sync::race_hook;
use crate::sync::test_support::{create_project, sync_project, write_file};
use crate::watch::WatcherHealth;
use std::fs;

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

/// A corrupt on-disk database must be discarded and rebuilt from scratch on
/// the next sync rather than erroring forever — the project stays usable.
/// Exercises `ensure_current`'s corruption-at-open branch (discard +
/// `publish_from_scratch`).
#[test]
fn a_corrupt_database_is_discarded_and_rebuilt_on_the_next_sync() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);

    // Replace the database with bytes that are not a SQLite database at
    // all, so the next open fails as corrupt.
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"this is not a sqlite database").expect("corrupt db");

    // ensure_current must discard the corrupt database and rebuild from
    // scratch instead of erroring — the project stays usable. (The rebuilt
    // database's first revision id is `rev-1` again: ids are per-database
    // rowids, so the revision id is not a uniqueness signal across
    // incarnations.)
    let _synced2 = sync_project(&manager, &project);
    let connection = open_db(&project);
    let content: Option<String> = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read content from the rebuilt database");
    assert!(
        content.unwrap().contains("pub fn a()"),
        "the rebuilt index must still serve the file's evidence"
    );
}

/// An unopenable database (the db path is a directory) must surface as an
/// error, never a panic, and never silently publish into nothing.
#[test]
fn an_unopenable_database_surfaces_as_an_error_not_a_panic() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let database = project.path().join(".planning/slugaudit/project.db");
    fs::create_dir_all(&database).expect("make the db path a directory");

    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    assert!(
        result.is_err(),
        "an unopenable database must error, not panic"
    );
}

/// When the watcher is Healthy and incremental reconcile fails, the
/// watcher must be marked `Desynced` (so the next call does a full
/// verification) and the failure surfaced — never a silent serve of stale
/// evidence. A path that turns into a directory between syncs makes the
/// re-hash fail deterministically without watcher-timing sleeps.
#[test]
fn a_failed_incremental_reconcile_marks_the_watcher_desynced() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    let file = project.path().join("lib.rs");
    fs::remove_file(&file).expect("remove file");
    fs::create_dir(&file).expect("replace lib.rs with a directory");

    let state = manager.active_watch_state().expect("registered state");
    state.mark_dirty("lib.rs".to_owned());

    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    assert!(
        result.is_err(),
        "a failed reconcile must surface as an error"
    );
    assert_eq!(
        state.health(),
        WatcherHealth::Desynced,
        "the watcher must be marked untrusted so the next call re-verifies"
    );
}
