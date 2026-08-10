//! Integration tests for SourceSyncManager.

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
        .ensure_current(&project.path().to_string_lossy())
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
