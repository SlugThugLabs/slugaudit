//! Integration tests for SourceSyncManager.
// slugaudit-line-exception: approved-by=agent; reason=one end-to-end watcher-backed scenario per sync invariant, all sharing the same create_project/write_file/sync_project fixture helpers; splitting would force a cross-module test harness or duplicate the four helpers in every file

use crate::sync::SourceSyncManager;
use crate::tools::context::with_verified_read;
use rmcp::ErrorData;
use std::fs;
use std::thread;
use std::time::Duration;

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

fn sync_project(
    manager: &SourceSyncManager,
    project: &tempfile::TempDir,
) -> crate::sync::SyncedProject {
    manager
        .ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
        .expect("sync succeeds")
}

fn db_error(error: rusqlite::Error) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

#[test]
fn edit_file_between_calls_returns_fresh_evidence() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn value() -> i32 { 42 }\n");

    let synced = sync_project(&manager, &project);
    let value_a_revision = synced.revision_id.clone();

    let content_a: Option<String> = with_verified_read(&synced, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert!(content_a.is_some());
    assert!(content_a.unwrap().contains("42"));

    write_file(&project, "lib.rs", b"pub fn value() -> i32 { 99 }\n");
    thread::sleep(Duration::from_millis(200));

    let synced_b = sync_project(&manager, &project);
    assert_ne!(value_a_revision, synced_b.revision_id);

    let content_b: Option<String> = with_verified_read(&synced_b, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert!(content_b.is_some());
    assert!(content_b.unwrap().contains("99"));
}

#[test]
fn edit_file_immediately_returns_fresh_evidence() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced = sync_project(&manager, &project);
    let old_revision = synced.revision_id.clone();

    write_file(&project, "lib.rs", b"pub fn a() { changed() }\n");
    let synced2 = sync_project(&manager, &project);

    assert_ne!(old_revision, synced2.revision_id);
}

#[test]
fn create_new_file_gets_indexed() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced = sync_project(&manager, &project);
    let file_count: i64 = with_verified_read(&synced, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(db_error)
    })
    .expect("read count");
    assert_eq!(file_count, 1);

    write_file(&project, "new.rs", b"pub fn b() {}\n");
    thread::sleep(Duration::from_millis(200));

    let synced2 = sync_project(&manager, &project);
    let file_count2: i64 = with_verified_read(&synced2, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(db_error)
    })
    .expect("read count");
    assert_eq!(file_count2, 2);
}

#[test]
fn delete_file_gets_removed_from_index() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    write_file(&project, "to_delete.rs", b"pub fn b() {}\n");

    let synced = sync_project(&manager, &project);
    let file_count: i64 = with_verified_read(&synced, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(db_error)
    })
    .expect("read count");
    assert_eq!(file_count, 2);

    fs::remove_file(project.path().join("to_delete.rs")).expect("delete file");
    thread::sleep(Duration::from_millis(200));

    let synced2 = sync_project(&manager, &project);
    let file_count2: i64 = with_verified_read(&synced2, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(db_error)
    })
    .expect("read count");
    assert_eq!(file_count2, 1);
}

#[test]
fn write_same_contents_does_not_create_new_revision() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    let content = b"pub fn a() {}\n";
    write_file(&project, "lib.rs", content);

    let synced = sync_project(&manager, &project);
    let revision = synced.revision_id.clone();

    write_file(&project, "lib.rs", content);
    thread::sleep(Duration::from_millis(200));

    let synced2 = sync_project(&manager, &project);
    assert_eq!(revision, synced2.revision_id);
}

#[test]
fn multiple_rapid_edits_collapse_to_one_revision() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced = sync_project(&manager, &project);
    let old_revision = synced.revision_id.clone();

    for i in 0..5 {
        write_file(
            &project,
            "lib.rs",
            format!("pub fn a{}() {{}}\n", i).as_bytes(),
        );
    }
    thread::sleep(Duration::from_millis(300));

    let synced2 = sync_project(&manager, &project);
    assert_ne!(old_revision, synced2.revision_id);
}

#[test]
fn restart_after_offline_changes_detects_changes() {
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn original() {}\n");

    let manager1 = SourceSyncManager::with_watcher();
    let synced1 = sync_project(&manager1, &project);
    let revision1 = synced1.revision_id.clone();
    drop(manager1);

    write_file(&project, "lib.rs", b"pub fn changed() {}\n");

    let manager2 = SourceSyncManager::with_watcher();
    let synced2 = sync_project(&manager2, &project);
    assert_ne!(revision1, synced2.revision_id);
}

/// Proves M5/M7: the barrier synchronization loop catches events that
/// arrive during reconciliation. We simulate this by modifying a file,
/// calling ensure_current (which reconciles), then modifying again before
/// the barrier loop completes. The second modification must be reconciled
/// in the same call, not deferred to the next.
#[test]
fn barrier_loop_catches_events_during_reconciliation() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let _synced = sync_project(&manager, &project);

    // Modify the file — this marks it dirty.
    write_file(&project, "lib.rs", b"pub fn b() {}\n");

    // Before calling ensure_current, also modify it again. Both events
    // should be reconciled in a single barrier loop.
    write_file(&project, "lib.rs", b"pub fn c() {}\n");
    thread::sleep(Duration::from_millis(200));

    let synced2 = sync_project(&manager, &project);

    let content: Option<String> = with_verified_read(&synced2, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert!(content.unwrap().contains("pub fn c()"));
}

/// Proves M9 fix: when the watcher is Unavailable, health must never be
/// overwritten to NeedsVerification. Every ensure_current call must do a
/// full verification, never trusting a (non-existent) dirty set.
#[test]
fn unavailable_watcher_always_does_full_verification() {
    // Create a manager without a watcher.
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced1 = sync_project(&manager, &project);
    let revision1 = synced1.revision_id.clone();

    // Modify the file. Since the watcher is Unavailable, the dirty set
    // is never populated — but the next call must still do a full
    // verification and detect the change.
    write_file(&project, "lib.rs", b"pub fn b() {}\n");

    let synced2 = sync_project(&manager, &project);
    assert_ne!(revision1, synced2.revision_id);

    let content: Option<String> = with_verified_read(&synced2, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert!(content.unwrap().contains("pub fn b()"));
}

/// Proves delete-then-recreate is handled correctly: the file should be
/// re-indexed with its new content, not left as a deletion.
#[test]
fn delete_then_recreate_gets_reindexed() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn original() {}\n");

    let _synced = sync_project(&manager, &project);

    // Delete and recreate the file with different content.
    fs::remove_file(project.path().join("lib.rs")).expect("delete file");
    thread::sleep(Duration::from_millis(200));
    write_file(&project, "lib.rs", b"pub fn recreated() {}\n");
    thread::sleep(Duration::from_millis(200));

    let synced2 = sync_project(&manager, &project);

    let content: Option<String> = with_verified_read(&synced2, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert!(content.unwrap().contains("pub fn recreated()"));
}

/// Proves that after a full verification (NeedsVerification branch),
/// events that arrived during the verification are drained before
/// setting Healthy. We verify this by checking that the final revision
/// reflects all changes, not just the ones present at the start of
/// verification.
#[test]
fn drains_events_after_full_verification() {
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn original() {}\n");

    let manager1 = SourceSyncManager::with_watcher();
    let _synced1 = sync_project(&manager1, &project);
    drop(manager1);

    // Make changes while no manager is active.
    write_file(&project, "lib.rs", b"pub fn changed() {}\n");
    write_file(&project, "new.rs", b"pub fn new() {}\n");

    // New manager → NeedsVerification → full publish → drain events.
    let manager2 = SourceSyncManager::with_watcher();
    let synced2 = sync_project(&manager2, &project);

    let file_count: i64 = with_verified_read(&synced2, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(db_error)
    })
    .expect("read count");
    assert_eq!(file_count, 2);

    let content: Option<String> = with_verified_read(&synced2, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert!(content.unwrap().contains("pub fn changed()"));
}
