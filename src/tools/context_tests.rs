use super::*;
use crate::tools::test_support::activated_project;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
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
    ensure(
        !rebuilt.revision_id.is_empty(),
        "!rebuilt.revision_id.is_empty()",
    )?;
    ensure(
        rebuilt.database_path.exists(),
        "rebuilt.database_path.exists()",
    )?;
    let connection = crate::store::open_existing(&rebuilt.database_path).check("rebuilt db")?;
    let count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .check("rebuilt file row")?;
    ensure_eq(&count, &1, "count == 1")?;
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
    ensure_eq(&count, &1_i64, "count == 1_i64")?;
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
    ensure_ne(
        &stale.revision_id,
        &fresh.revision_id,
        "stale.revision_id != fresh.revision_id",
    )?;

    let ran_closure = std::cell::Cell::new(false);
    let result = with_verified_read(&stale, |_tx| {
        ran_closure.set(true);
        Ok(())
    });
    let error = result.check_err("a stale revision handle must never open against newer data")?;
    ensure(!ran_closure.get(), "!ran_closure.get()")?;
    // The message must actually be actionable, not a bare rusqlite/io
    // Display string with no guidance — this is the specific caller-facing
    // gap `internal_error` exists to close.
    let message = error.message.to_string();
    ensure(
        message.contains("retry the call"),
        "message.contains(\"retry the call\")",
    )?;
    // And the machine-readable half: a stale revision keeps the retry
    // advice *as a code*, so a caller can branch on it without reading the
    // message. This is the one failure where "retry" is the right answer,
    // which is exactly why it must be distinguishable from the sync budget
    // (whose remedy says the opposite).
    let data = error.data.check("a stale revision carries a payload")?;
    ensure_eq(
        data.get("code").check("data[\"code\"]")?,
        &"stale_revision",
        "data[\"code\"] == \"stale_revision\"",
    )?;
    let remedy = data
        .get("remedy")
        .check("data[\"remedy\"]")?
        .as_str()
        .check("remedy is a string")?;
    ensure(
        remedy.contains("retry the call"),
        "remedy.contains(\"retry the call\")",
    )?;
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
    ensure(
        !rebuilt.revision_id.is_empty(),
        "!rebuilt.revision_id.is_empty()",
    )?;
    let connection = crate::store::open_existing(&rebuilt.database_path).check("rebuilt db")?;
    let stored_root: String = connection
        .query_row("SELECT root_path FROM project WHERE id = 1", [], |row| {
            row.get(0)
        })
        .check("read stored root")?;
    ensure_ne(
        &stored_root,
        &"/some/other/project/root",
        "stored_root != \"/some/other/project/root\"",
    )?;
    let count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .check("rebuilt file row")?;
    ensure_eq(&count, &1, "count == 1")?;
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

    ensure_eq(
        &first.revision_id,
        &second.revision_id,
        "first.revision_id == second.revision_id",
    )?;
    Ok(())
}
