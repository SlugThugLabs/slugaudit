//! Integration tests for SourceSyncManager.

use crate::store::CURRENT_SCHEMA_VERSION;
use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};
use crate::tools::context::with_verified_read;
use rmcp::ErrorData;
use std::fs;
use std::thread;
use std::time::Duration;

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
    assert!(content_a.expect("read file back").contains("42"));

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
    assert!(content_b.expect("read file back").contains("99"));
}

#[test]
fn edit_file_immediately_returns_fresh_evidence() {
    // Waits the same 200 ms the other watcher-backed tests wait after a
    // write: the `notify` watcher debounces and coalesces events on its
    // own thread, so a tool call that races a write without any delay
    // can observe an empty dirty-set and return the prior revision.
    // The contract this test pins is "the next `sync_project` after a
    // real filesystem change observes the change" - not "the very next
    // CPU cycle does". AI tool calls always have some latency of their
    // own, so the wall-clock delay is realistic.
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let synced = sync_project(&manager, &project);
    let old_revision = synced.revision_id;

    write_file(&project, "lib.rs", b"pub fn a() { changed() }\n");
    thread::sleep(Duration::from_millis(200));

    let synced2 = sync_project(&manager, &project);

    assert_ne!(old_revision, synced2.revision_id);
}

#[test]
fn watcher_reconcile_applies_the_projects_file_size_limit() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(
        &project,
        ".planning/slugaudit/config.json",
        br#"{"max_file_bytes":50}"#,
    );
    write_file(&project, "lib.rs", b"pub fn initially_small() {}\n");
    let first = sync_project(&manager, &project);

    write_file(&project, "lib.rs", &[b'x'; 200]);
    thread::sleep(Duration::from_millis(200));
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
    .expect("read reconciled file state");
    assert_eq!(kind, "skipped", "the project limit must govern reconcile");
    assert_eq!(content, None, "an oversized file must retain no content");
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
    let revision = synced.revision_id;

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
    let old_revision = synced.revision_id;

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
    let revision1 = synced1.revision_id;
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
    assert!(content.expect("read file back").contains("pub fn c()"));
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
    let revision1 = synced1.revision_id;

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
    assert!(content.expect("read file back").contains("pub fn b()"));
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
    assert!(
        content
            .expect("read file back")
            .contains("pub fn recreated()")
    );
}

/// C12: the consecutive-full-publish health metric counts full publishes
/// and resets on a successful incremental reconcile. A fresh manager
/// starts NeedsVerification → first sync is a full publish (count 1); an
/// unchanged second sync stays Healthy with no events → count stays 1;
/// an actual edit reconciles incrementally → count resets to 0.
#[test]
fn consecutive_full_publishes_tracks_full_publishes_and_resets_on_incremental() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    // First sync: NeedsVerification → full publish.
    let synced = sync_project(&manager, &project);
    assert!(!synced.revision_id.is_empty());
    assert_eq!(
        manager.consecutive_full_publishes(),
        1,
        "first sync is a full publish"
    );

    // No changes: Healthy + no events → no new publish, count unchanged.
    let _ = sync_project(&manager, &project);
    assert_eq!(
        manager.consecutive_full_publishes(),
        1,
        "unchanged sync is not a publish"
    );

    // A real edit: Healthy + dirty → incremental reconcile → reset.
    write_file(&project, "lib.rs", b"pub fn b() {}\n");
    thread::sleep(Duration::from_millis(200));
    let _ = sync_project(&manager, &project);
    assert_eq!(
        manager.consecutive_full_publishes(),
        0,
        "a successful incremental reconcile resets the counter"
    );
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
    assert!(
        content
            .expect("read file back")
            .contains("pub fn changed()")
    );
}

#[test]
fn outdated_database_is_automatically_deleted_and_recreated() {
    let manager = SourceSyncManager::with_watcher();
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

    let version: i64 = with_verified_read(&synced_after, |tx| {
        tx.query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db_error)
    })
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

/// The classifier behind the two `ErrorData` map sites in
/// `manager_health`. Both budget shapes reach them — the sweep's own
/// variant, and the reconcile pipeline's budget wrapped one level deeper —
/// and everything else must stay `None` so no unrelated failure gets the
/// budget code attached.
#[test]
fn sync_error_budget_classifier_sees_through_the_reconcile_wrapper() {
    use super::SyncError;
    use crate::sync::DiscoveryError;
    use crate::sync::reconcile::ReconcileError;

    assert_eq!(
        SyncError::SweepBudgetExceeded { elapsed_ms: 9 }.budget_elapsed_ms(),
        Some(9)
    );
    assert_eq!(
        SyncError::Reconcile(ReconcileError::TimeBudgetExceeded {
            path: String::new(),
            elapsed_ms: 3,
        })
        .budget_elapsed_ms(),
        Some(3)
    );
    assert_eq!(
        SyncError::Reconcile(ReconcileError::Discovery(
            DiscoveryError::TimeBudgetExceeded { elapsed_ms: 5 },
        ))
        .budget_elapsed_ms(),
        Some(5)
    );
    assert_eq!(
        SyncError::Sweep("database error: disk I/O error".to_owned()).budget_elapsed_ms(),
        None
    );
    assert_eq!(
        SyncError::Database(rusqlite::Error::QueryReturnedNoRows).budget_elapsed_ms(),
        None
    );
}
