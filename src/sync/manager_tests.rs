//! Integration tests for `SourceSyncManager`'s full-scan synchronization:
//! every call hashes the tree, so a change is observed on the next call with
//! no watcher and no delay.

use crate::store::CURRENT_SCHEMA_VERSION;
use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};
use crate::tools::context::with_verified_read;
use rmcp::ErrorData;
use std::fs;

fn db_error(error: rusqlite::Error) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

#[test]
fn edit_file_between_calls_returns_fresh_evidence() {
    let manager = SourceSyncManager::new();
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
    assert!(content_a.expect("read file back").contains("42"));

    write_file(&project, "lib.rs", b"pub fn value() -> i32 { 99 }\n");

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
    assert!(content_b.expect("read file back").contains("99"));
}

#[test]
fn edit_file_immediately_returns_fresh_evidence() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced = sync_project(&manager, &project);
    let old_revision = synced.revision_id;

    write_file(&project, "lib.rs", b"pub fn a() { changed() }\n");

    let synced2 = sync_project(&manager, &project);

    assert_ne!(old_revision, synced2.revision_id);
}

#[test]
fn project_file_size_limit_governs_the_scan() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(
        &project,
        ".planning/slugaudit/config.json",
        br#"{"max_file_bytes":50}"#,
    );
    write_file(&project, "lib.rs", b"pub fn initially_small() {}\n");
    let first = sync_project(&manager, &project);

    write_file(&project, "lib.rs", &[b'x'; 200]);
    let second = sync_project(&manager, &project);

    assert_ne!(first.revision_id, second.revision_id);
    let (kind, content): (String, Option<String>) = with_verified_read(&second, |tx| {
        tx.query_row(
            "SELECT file_kind, content FROM files WHERE path = 'lib.rs'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_error)
    })
    .expect("read scanned file state");
    assert_eq!(kind, "skipped", "the project limit must govern the scan");
    assert_eq!(content, None, "an oversized file must retain no content");
}

#[test]
fn create_new_file_gets_indexed() {
    let manager = SourceSyncManager::new();
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
    let manager = SourceSyncManager::new();
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
    let manager = SourceSyncManager::new();
    let project = create_project();
    let content = b"pub fn a() {}\n";
    write_file(&project, "lib.rs", content);

    let synced = sync_project(&manager, &project);
    let revision = synced.revision_id;

    write_file(&project, "lib.rs", content);

    let synced2 = sync_project(&manager, &project);
    assert_eq!(revision, synced2.revision_id);
}

#[test]
fn multiple_rapid_edits_collapse_to_one_revision() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced = sync_project(&manager, &project);
    let old_revision = synced.revision_id;

    for i in 0..5 {
        write_file(
            &project,
            "lib.rs",
            format!("pub fn a{}() {{}}\n", i).as_bytes(),
        );
    }

    let synced2 = sync_project(&manager, &project);
    assert_ne!(old_revision, synced2.revision_id);
}

#[test]
fn restart_after_offline_changes_detects_changes() {
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn original() {}\n");

    let manager1 = SourceSyncManager::new();
    let synced1 = sync_project(&manager1, &project);
    let revision1 = synced1.revision_id;
    drop(manager1);

    write_file(&project, "lib.rs", b"pub fn changed() {}\n");

    let manager2 = SourceSyncManager::new();
    let synced2 = sync_project(&manager2, &project);
    assert_ne!(revision1, synced2.revision_id);
}

/// Proves delete-then-recreate is handled correctly: the file should be
/// re-indexed with its new content, not left as a deletion.
#[test]
fn delete_then_recreate_gets_reindexed() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn original() {}\n");

    let _synced = sync_project(&manager, &project);

    // Delete and recreate the file with different content.
    fs::remove_file(project.path().join("lib.rs")).expect("delete file");
    write_file(&project, "lib.rs", b"pub fn recreated() {}\n");

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
    assert!(
        content
            .expect("read file back")
            .contains("pub fn recreated()")
    );
}

/// The database-side proof: content edited directly in the database, with
/// the hash column left alone, is repaired from the file on the next call.
#[test]
fn a_content_edit_in_the_database_is_repaired_on_the_next_call() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let synced = sync_project(&manager, &project);
    let revision_before = synced.revision_id.clone();

    {
        let connection = rusqlite::Connection::open(&synced.database_path).expect("raw open");
        connection
            .execute(
                "UPDATE files SET content = 'fn tampered() {}\n' WHERE path = 'lib.rs'",
                [],
            )
            .expect("tamper with the stored content");
    }

    let synced2 = sync_project(&manager, &project);

    assert_ne!(
        revision_before, synced2.revision_id,
        "the repair publishes a revision"
    );
    let content: Option<String> = with_verified_read(&synced2, |tx| {
        tx.query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)
    })
    .expect("read content");
    assert_eq!(
        content.as_deref(),
        Some("pub fn a() {}\n"),
        "the source wins over the database"
    );
}

#[test]
fn outdated_database_is_automatically_deleted_and_recreated() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn first() {}\n");

    // First sync to establish the initial database.
    let synced = sync_project(&manager, &project);
    assert!(!synced.revision_id.is_empty());

    // Artificially stamp an outdated schema version into user_version (simulating an older version).
    {
        let connection = rusqlite::Connection::open(&synced.database_path).expect("raw open");
        connection
            .pragma_update(None, "user_version", 1_i64)
            .expect("rewind user_version to 1");
    }

    // Next sync must detect the outdated version, discard the old DB, and create a fresh new one.
    let synced_after = sync_project(&manager, &project);
    assert!(!synced_after.revision_id.is_empty());

    // Read the version through a raw connection, the same way it was stamped
    // above, so the check does not depend on the store's connection contract.
    let version: i64 = rusqlite::Connection::open(&synced_after.database_path)
        .expect("raw open")
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read version");
    assert_eq!(
        version, CURRENT_SCHEMA_VERSION,
        "recreated database must be at current schema version"
    );

    let file_count: i64 = with_verified_read(&synced_after, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(db_error)
    })
    .expect("read count");
    assert_eq!(
        file_count, 1,
        "source files must be republished from scratch"
    );
}
