//! Database discard and rebuild recovery tests for `SourceSyncManager`.

use super::*;
use crate::progress::NoopProgressSink;
use crate::store::CURRENT_SCHEMA_VERSION;
use crate::sync::test_support::{create_project, sync_project, write_file};
use crate::tools::context::with_verified_read;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

fn stored_file_count(synced: &SyncedProject) -> TestResult<i64> {
    with_verified_read(synced, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .check("read file count")
}

#[test]
fn outdated_database_is_automatically_deleted_and_recreated() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn first() {}\n")?;

    let synced = sync_project(&manager, &project)?;
    ensure(
        !synced.revision_id.is_empty(),
        "!synced.revision_id.is_empty()",
    )?;

    {
        let connection = rusqlite::Connection::open(&synced.database_path).check("raw open")?;
        connection
            .pragma_update(None, "user_version", 1_i64)
            .check("rewind user_version to 1")?;
    }

    let synced_after = sync_project(&manager, &project)?;
    ensure(
        !synced_after.revision_id.is_empty(),
        "!synced_after.revision_id.is_empty()",
    )?;

    let version: i64 = rusqlite::Connection::open(&synced_after.database_path)
        .check("raw open")?
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .check("read version")?;
    ensure_eq(
        &version,
        &CURRENT_SCHEMA_VERSION,
        "recreated database must be at current schema version",
    )?;
    ensure_eq(
        &stored_file_count(&synced_after)?,
        &1,
        "source files must be republished from scratch",
    )?;
    Ok(())
}

#[test]
fn database_corrupted_during_sync_is_automatically_rebuilt() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn first() {}\n")?;

    let synced = sync_project(&manager, &project)?;
    ensure(
        !synced.revision_id.is_empty(),
        "!synced.revision_id.is_empty()",
    )?;

    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&synced.database_path)
            .check("open db file")?;
        file.seek(SeekFrom::Start(0)).check("seek")?;
        file.write_all(b"not a valid sqlite database header at all!")
            .check("corrupt header")?;
    }

    let synced_after = sync_project(&manager, &project)?;
    ensure(
        !synced_after.revision_id.is_empty(),
        "!synced_after.revision_id.is_empty()",
    )?;
    synced_after
        .discard
        .as_ref()
        .check("`synced_after.discard` is present")?;
    ensure_eq(
        &stored_file_count(&synced_after)?,
        &1,
        "source files must be republished from scratch after corruption",
    )?;
    Ok(())
}

#[test]
fn database_corrupted_during_publish_is_automatically_rebuilt() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn first() {}\n")?;

    let synced = sync_project(&manager, &project)?;
    ensure(
        !synced.revision_id.is_empty(),
        "!synced.revision_id.is_empty()",
    )?;

    write_file(&project, "second.rs", b"pub fn second() {}\n")?;

    let db_path = synced.database_path.clone();
    let hook = super::super::race_hook::set(
        project.path(),
        super::super::race_hook::RacePoint::BeforeCommit,
        move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&db_path)
                .check("open db file")?;
            file.set_len(512).check("truncate db")?;
            file.sync_all().check("sync")
        },
    )?;

    let synced_after = sync_project(&manager, &project)?;
    hook.outcome()?;
    ensure(
        !synced_after.revision_id.is_empty(),
        "!synced_after.revision_id.is_empty()",
    )?;
    let discard = synced_after
        .discard
        .as_ref()
        .check("discard notice expected")?;
    ensure(
        discard.cause.contains("corrupt"),
        format!(
            "discard notice should mention corruption: {}",
            discard.cause
        ),
    )?;
    ensure_eq(
        &stored_file_count(&synced_after)?,
        &2,
        "both files must be published after rebuilding from corruption",
    )?;
    Ok(())
}

#[test]
fn a_failed_stale_database_discard_reports_the_exact_error() -> TestResult {
    let project = create_project()?;
    let root = project::ProjectRoot::resolve(project.path()).check("project root")?;
    let database_path = project::database_path(&root);
    let connection = store::open_read_write(&database_path).check("open database")?;
    connection
        .execute(
            "INSERT INTO project VALUES (1, 'default', '/elsewhere', 1, 1, 0)",
            [],
        )
        .check("stale metadata row")?;
    fs::remove_file(&database_path).check("unlink the database file")?;
    fs::create_dir(&database_path).check("directory at the database path")?;
    let manager = SourceSyncManager::default();

    let Err(error) = manager.verify_metadata_or_rebuild(
        &root,
        &database_path,
        connection,
        std::time::Instant::now(),
        &NoopProgressSink,
    ) else {
        return Err("the directory at the database path cannot be discarded".to_owned())
            .check("unexpected outcome");
    };

    let discard_error = fs::remove_file(&database_path)
        .check_err("the directory at the database path must not unlink")?;
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "failed discard reports INTERNAL_ERROR",
    )?;
    ensure_eq(
        &error.message,
        &format!("discarding the stale project database: {discard_error}"),
        "failed discard preserves underlying unlink error",
    )?;
    Ok(())
}
