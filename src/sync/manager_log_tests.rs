//! Log-content tests for the manager's database-recovery records: a discard
//! or open failure must carry static categories and the real database path
//! (the real-path logging policy), including a stored root from
//! another machine or location.

use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

/// The structured records for one sync-log operation, one per failure.
fn sync_failure_records<'a>(logs: &'a str, operation: &str) -> Vec<&'a str> {
    let marker = format!("operation=\"{operation}\"");
    logs.lines().filter(|line| line.contains(&marker)).collect()
}

/// R12: the stale-metadata warning must not format a reason string that
/// embeds the *previous* absolute project root (the move/copy/unzip
/// scenario). The record must carry a static kind, the real database path,
/// and the old root as a real path.
#[test]
fn a_stale_project_root_discard_warn_logs_real_path() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn a() {}\n")?;
    let _synced = sync_project(&manager, &project)?;

    let database = project.path().join(".planning/slugaudit/project.db");
    let stale_root = format!(
        "{}-copied-from-elsewhere",
        project
            .path()
            .canonicalize()
            .check("canonicalize project tempdir")?
            .display()
    );
    {
        let connection = crate::store::open_read_write(&database).check("open db")?;
        connection
            .execute(
                "UPDATE project SET root_path = ?1 WHERE id = 1",
                [&stale_root],
            )
            .check("rewrite stored root")?;
    }
    let expected_path = database.display().to_string();

    let (synced, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for verify_project_database");
        sync_project(&manager, &project)
    })?;
    synced?;

    ensure(
        logs.contains("capture liveness control for verify_project_database"),
        format!("the capture must see this thread's events, captured: {logs}"),
    )?;
    let records = sync_failure_records(&logs, "verify_project_database");
    ensure_eq(
        &records.len(),
        &1,
        format!("one failure is one record:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains("error_kind=\"root_mismatch\""),
        format!("the record must carry the static reason category:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains(&format!("path={expected_path}")),
        format!("the discard WARN must carry the real database path:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains(&format!("stored_root={stale_root}")),
        format!("the old root must be logged as a real path:\n{logs}"),
    )?;
    Ok(())
}

/// R12: the open-failure warning must not format `StoreError`, whose
/// dependency errors can embed the database path. The record must carry a
/// static kind and the real database path.
#[test]
fn a_database_open_failure_warn_logs_real_path() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    let database = project.path().join(".planning/slugaudit/project.db");
    // A regular file keeps the project active; a directory at the `-wal`
    // sidecar path is what makes the SQLite connection setup fail.
    fs::create_dir_all(database.with_extension("db-wal")).check("make the wal path a directory")?;
    // The failure is the SQLite connection setup (`StoreError::Configure`).
    let expected_path = database.display().to_string();

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for open_project_database");
        manager.ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
    })?;

    ensure(result.is_err(), "the open failure must surface")?;
    ensure(
        logs.contains("capture liveness control for open_project_database"),
        format!("the capture must see this thread's events, captured: {logs}"),
    )?;
    let records = sync_failure_records(&logs, "open_project_database");
    ensure_eq(
        &records.len(),
        &1,
        format!("one failure is one record:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains("error_kind=\"configure\""),
        format!("the record must carry the static error category:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains(&format!("path={expected_path}")),
        format!("the WARN must carry the real database path:\n{logs}"),
    )?;
    Ok(())
}

/// An `OpenCorrupt` recovery record: the real database path and the source
/// error text reach the WARN.
#[test]
fn an_open_corrupt_recovery_warn_logs_real_path_and_source_error() -> TestResult {
    use super::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let error = crate::store::StoreError::Missing;
    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(database, &DatabaseRecovery::OpenCorrupt { error: &error });
    })?;

    let records = sync_failure_records(&logs, "open_project_database");
    ensure_eq(
        &records.len(),
        &1,
        format!("one recovery is one record:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains(&format!("path={}", database.display())),
        format!("the record must carry the real database path:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains("error_kind=\"missing\"")
            && records
                .first()
                .check("records[0] exists")?
                .contains(&format!("error={error}")),
        format!("the record must carry the category and the source error:\n{logs}"),
    )?;
    ensure(
        !records
            .first()
            .check("records[0] exists")?
            .contains("database_id="),
        format!("the redacted database id is gone:\n{logs}"),
    )?;
    Ok(())
}

/// An `IncompatibleVersion` recovery record carries the real database path,
/// and both versions.
#[test]
fn an_incompatible_version_recovery_warn_logs_real_path_and_versions() -> TestResult {
    use super::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(
            database,
            &DatabaseRecovery::IncompatibleVersion {
                operation: "verify_project_database",
                contract_version: 7,
                schema_version: 9,
            },
        );
    })?;

    let records = sync_failure_records(&logs, "verify_project_database");
    ensure_eq(
        &records.len(),
        &1,
        format!("one recovery is one record:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains(&format!("path={}", database.display())),
        format!("the record must carry the real database path:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains("error_kind=\"incompatible_version\"")
            && records
                .first()
                .check("records[0] exists")?
                .contains("contract_version=7")
            && records
                .first()
                .check("records[0] exists")?
                .contains("schema_version=9"),
        format!("the record must carry the category and versions:\n{logs}"),
    )?;
    ensure(
        !records
            .first()
            .check("records[0] exists")?
            .contains("database_id="),
        format!("the redacted database id is gone:\n{logs}"),
    )?;
    Ok(())
}

/// The open-failure (non-discarding) record also carries the source error.
#[test]
fn an_open_failed_recovery_warn_logs_the_source_error() -> TestResult {
    use super::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let error = crate::store::StoreError::Missing;

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(database, &DatabaseRecovery::OpenFailed { error: &error });
    })?;

    let records = sync_failure_records(&logs, "open_project_database");
    ensure_eq(
        &records.len(),
        &1,
        format!("one failure is one record:\n{logs}"),
    )?;
    ensure(
        records
            .first()
            .check("records[0] exists")?
            .contains(&format!("error={error}")),
        format!("the record must carry the source error:\n{logs}"),
    )?;
    Ok(())
}

/// A database with no revision row makes `revision_after_sync` return an
/// internal error naming the real database path, and log nothing itself:
/// `server_runner::log_completion` logs the returned message once.
#[test]
fn a_missing_revision_error_names_the_real_database_path() -> TestResult {
    let directory = tempfile::tempdir().check("db dir")?;
    let database = directory.path().join("project.db");
    let connection = crate::store::open_read_write(&database).check("open db")?;

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        crate::sync::project_meta::revision_after_sync(&connection, &database)
    })?;

    let error = result.err().check("no revision must surface as an error")?;
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure(
        error.message.starts_with(&format!(
            "no current revision found after sync in {}",
            database.display()
        )),
        format!(
            "the error must carry the real database path: {}",
            error.message
        ),
    )?;
    ensure(
        logs.is_empty(),
        format!("a returned failure is not logged first:\n{logs}"),
    )?;
    Ok(())
}

/// A failed revision read makes `revision_after_sync` return an internal
/// error naming the real database path and the SQLite cause, and log
/// nothing itself.
#[test]
fn a_failed_revision_read_error_names_the_database_path_and_cause() -> TestResult {
    let directory = tempfile::tempdir().check("db dir")?;
    let database = directory.path().join("project.db");
    let connection = crate::store::open_read_write(&database).check("open db")?;
    connection
        .execute_batch("DROP TABLE revisions")
        .check("drop the revisions table")?;

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        crate::sync::project_meta::revision_after_sync(&connection, &database)
    })?;

    let error = result
        .err()
        .check("a failed read must surface as an error")?;
    ensure(
        error.message.starts_with(&format!(
            "reading the current revision of {}: ",
            database.display()
        )),
        format!("{}", error.message),
    )?;
    ensure(
        error.message.contains("revisions"),
        format!("{}", error.message),
    )?;
    ensure(
        logs.is_empty(),
        format!("a returned failure is not logged first:\n{logs}"),
    )?;
    Ok(())
}
