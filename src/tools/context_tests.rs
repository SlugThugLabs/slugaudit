use super::*;
use crate::tools::test_support::activated_project;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::fs;

#[test]
fn a_corrupt_derived_database_is_discarded_and_rebuilt_from_project_files() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn rebuilt() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();
    ensure_synced_no_progress(&path, &crate::sync::SourceSyncManager::default())
        .check("initial sync")?;

    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"corrupt sqlite bytes").check("corrupt derived database")?;
    fs::write(
        database.with_file_name("project.db-wal"),
        b"stale wal sidecar",
    )
    .check("write stale wal sidecar")?;

    let rebuilt = ensure_synced_no_progress(&path, &crate::sync::SourceSyncManager::default())
        .check("corrupt cache is rebuilt")?;
    assert!(!rebuilt.revision_id.is_empty());
    assert!(rebuilt.database_path.exists(), "rebuilt database exists");
    let connection = crate::store::open_existing(&rebuilt.database_path).check("rebuilt db")?;
    let count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .check("rebuilt file row")?;
    assert_eq!(count, 1);
    Ok(())
}

/// Deterministic sync for the freshness tests: every `ensure_current`
/// hashes the tree, so a content change always moves the revision and no
/// background event timing can turn a "stale handle must be rejected"
/// assertion into a flaky one.
fn synced_locally(path: &str) -> TestResult<SyncedProject> {
    crate::sync::SourceSyncManager::new()
        .ensure_current(path, &crate::progress::NoopProgressSink)
        .check("local sync")
}

/// Exercises the real production entry point tools actually call, not a
/// weaker stand-in: `with_verified_read` keeps the transaction open for the
/// whole closure, so a real query issued from inside `f` proves verification
/// and the read share one atomic snapshot end to end.
#[test]
fn a_verified_connection_succeeds_when_nothing_changed_since_sync() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let synced = ensure_synced_no_progress(
        &project.path().to_string_lossy(),
        &crate::sync::SourceSyncManager::default(),
    )
    .check("sync")?;

    let count: i64 = with_verified_read(&synced, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .check("still-fresh revision opens fine and the closure's query runs")?;
    assert_eq!(count, 1_i64);
    Ok(())
}

#[test]
fn a_stale_synced_handle_fails_loudly_instead_of_returning_mismatched_data() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();
    let stale = synced_locally(&path)?;

    // Simulate a concurrent publish from another process: modify the file
    // and sync again independently of `stale`.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .check("modify file")?;
    let fresh = synced_locally(&path)?;
    assert_ne!(
        stale.revision_id, fresh.revision_id,
        "the revision must actually have moved"
    );

    let ran_closure = std::cell::Cell::new(false);
    let result = with_verified_read(&stale, |_tx| {
        ran_closure.set(true);
        Ok(())
    });
    let error = result.check_err("a stale revision handle must never open against newer data")?;
    assert!(
        !ran_closure.get(),
        "the closure must never run against a revision that failed verification"
    );
    // The message must actually be actionable, not a bare rusqlite/io
    // Display string with no guidance — this is the specific caller-facing
    // gap `internal_error` exists to close.
    let message = error.message.to_string();
    assert!(
        message.contains("retry the call"),
        "expected a retry hint in the error message, got: {message}"
    );
    // And the machine-readable half: a stale revision keeps the retry
    // advice *as a code*, so a caller can branch on it without reading the
    // message. This is the one failure where "retry" is the right answer,
    // which is exactly why it must be distinguishable from the sync budget
    // (whose remedy says the opposite).
    let data = error.data.check("a stale revision carries a payload")?;
    assert_eq!(data["code"], "stale_revision");
    let remedy = data["remedy"].as_str().check("remedy is a string")?;
    assert!(
        remedy.contains("retry the call"),
        "the stale-revision remedy must keep the retry advice: {remedy}"
    );
    Ok(())
}

#[test]
fn a_database_copied_from_a_different_project_root_is_rebuilt_not_served() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();
    ensure_synced_no_progress(&path, &crate::sync::SourceSyncManager::default())
        .check("first sync establishes the project row")?;

    // Simulate a database file copied in from a different project: rewrite
    // the stored root_path directly through a raw connection, bypassing
    // ensure_synced entirely.
    let database_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let raw = rusqlite::Connection::open(&database_path).check("open raw connection")?;
    raw.execute(
        "UPDATE project SET root_path = ?1 WHERE id = 1",
        rusqlite::params!["/some/other/project/root"],
    )
    .check("simulate a copied database from another project")?;
    drop(raw);

    // The stale database must never be silently served; instead it is
    // discarded and rebuilt from this project's source, so the call succeeds
    // and the rebuilt index reflects THIS project's files, not the foreign
    // database's evidence.
    let rebuilt = ensure_synced_no_progress(&path, &crate::sync::SourceSyncManager::default())
        .check("a stale root must be rebuilt, not served or rejected")?;
    assert!(!rebuilt.revision_id.is_empty());
    let connection = crate::store::open_existing(&rebuilt.database_path).check("rebuilt db")?;
    let stored_root: String = connection
        .query_row("SELECT root_path FROM project WHERE id = 1", [], |row| {
            row.get(0)
        })
        .check("read stored root")?;
    assert_ne!(
        stored_root, "/some/other/project/root",
        "the rebuilt database must be bound to this project, not the foreign root"
    );
    let count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .check("rebuilt file row")?;
    assert_eq!(count, 1, "rebuilt from this project's source");
    Ok(())
}

/// Two syncs against an unchanged project converge on the same revision —
/// the publish is deterministic.
#[test]
fn two_syncs_against_unchanged_project_converge_on_same_revision() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();

    let first = ensure_synced_no_progress(&path, &crate::sync::SourceSyncManager::default())
        .check("first sync")?;
    let second = ensure_synced_no_progress(&path, &crate::sync::SourceSyncManager::default())
        .check("second sync")?;

    assert_eq!(
        first.revision_id, second.revision_id,
        "two full publishes against an unchanged project must converge on the same revision"
    );
    Ok(())
}
