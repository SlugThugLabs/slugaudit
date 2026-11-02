//! Log-content tests for the manager's database-recovery records: a discard
//! or open failure must carry static categories and the real database path
//! (the real-path logging policy), including a stored root from
//! another machine or location.

use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};
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
fn a_stale_project_root_discard_warn_logs_real_path() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);

    let database = project.path().join(".planning/slugaudit/project.db");
    let stale_root = format!(
        "{}-copied-from-elsewhere",
        project
            .path()
            .canonicalize()
            .expect("canonicalize project tempdir")
            .display()
    );
    {
        let connection = crate::store::open_read_write(&database).expect("open db");
        connection
            .execute(
                "UPDATE project SET root_path = ?1 WHERE id = 1",
                [&stale_root],
            )
            .expect("rewrite stored root");
    }
    let expected_path = database.display().to_string();

    let (_synced, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for verify_project_database");
        sync_project(&manager, &project)
    });

    assert!(
        logs.contains("capture liveness control for verify_project_database"),
        "the capture must see this thread's events, captured: {logs}"
    );
    let records = sync_failure_records(&logs, "verify_project_database");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("error_kind=\"root_mismatch\""),
        "the record must carry the static reason category:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("path={expected_path}")),
        "the discard WARN must carry the real database path:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("stored_root={stale_root}")),
        "the old root must be logged as a real path:\n{logs}"
    );
}

/// R12: the open-failure warning must not format `StoreError`, whose
/// dependency errors can embed the database path. The record must carry a
/// static kind and the real database path.
#[test]
fn a_database_open_failure_warn_logs_real_path() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    let database = project.path().join(".planning/slugaudit/project.db");
    // A regular file keeps the project active; a directory at the `-wal`
    // sidecar path is what makes the SQLite connection setup fail.
    fs::create_dir_all(database.with_extension("db-wal")).expect("make the wal path a directory");
    // The failure is the SQLite connection setup (`StoreError::Configure`).
    let expected_path = database.display().to_string();

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for open_project_database");
        manager.ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
    });

    assert!(result.is_err(), "the open failure must surface");
    assert!(
        logs.contains("capture liveness control for open_project_database"),
        "the capture must see this thread's events, captured: {logs}"
    );
    let records = sync_failure_records(&logs, "open_project_database");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("error_kind=\"configure\""),
        "the record must carry the static error category:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("path={expected_path}")),
        "the WARN must carry the real database path:\n{logs}"
    );
}

/// An `OpenCorrupt` recovery record: the real database path and the source
/// error text reach the WARN.
#[test]
fn an_open_corrupt_recovery_warn_logs_real_path_and_source_error() {
    use crate::sync::recovery_log::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let error = crate::store::StoreError::Symlink;
    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(database, DatabaseRecovery::OpenCorrupt { error: &error });
    });

    let records = sync_failure_records(&logs, "open_project_database");
    assert_eq!(records.len(), 1, "one recovery is one record:\n{logs}");
    assert!(
        records[0].contains(&format!("path={}", database.display())),
        "the record must carry the real database path:\n{logs}"
    );
    assert!(
        records[0].contains("error_kind=\"symlink\"")
            && records[0].contains(&format!("error={error}")),
        "the record must carry the category and the source error:\n{logs}"
    );
    assert!(
        !records[0].contains("database_id="),
        "the redacted database id is gone:\n{logs}"
    );
}

/// An `IncompatibleVersion` recovery record carries the real database path,
/// and both versions.
#[test]
fn an_incompatible_version_recovery_warn_logs_real_path_and_versions() {
    use crate::sync::recovery_log::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(
            database,
            DatabaseRecovery::IncompatibleVersion {
                contract_version: 7,
                schema_version: 9,
            },
        );
    });

    let records = sync_failure_records(&logs, "verify_project_database");
    assert_eq!(records.len(), 1, "one recovery is one record:\n{logs}");
    assert!(
        records[0].contains(&format!("path={}", database.display())),
        "the record must carry the real database path:\n{logs}"
    );
    assert!(
        records[0].contains("error_kind=\"incompatible_version\"")
            && records[0].contains("contract_version=7")
            && records[0].contains("schema_version=9"),
        "the record must carry the category and versions:\n{logs}"
    );
    assert!(
        !records[0].contains("database_id="),
        "the redacted database id is gone:\n{logs}"
    );
}

/// The open-failure (non-discarding) record also carries the source error.
#[test]
fn an_open_failed_recovery_warn_logs_the_source_error() {
    use crate::sync::recovery_log::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let error = crate::store::StoreError::Symlink;

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(database, DatabaseRecovery::OpenFailed { error: &error });
    });

    let records = sync_failure_records(&logs, "open_project_database");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains(&format!("error={error}")),
        "the record must carry the source error:\n{logs}"
    );
}

/// A database with no revision row makes `revision_after_sync` WARN with the
/// real database path (and return the internal error).
#[test]
fn a_missing_revision_warn_logs_the_real_database_path() {
    let directory = tempfile::tempdir().expect("db dir");
    let database = directory.path().join("project.db");
    let connection = crate::store::open_read_write(&database).expect("open db");

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        crate::sync::manager_meta::revision_after_sync(&connection, &database)
    });

    assert!(result.is_err(), "no revision must surface as an error");
    assert!(
        logs.contains("no current revision found after sync")
            && logs.contains(&format!("path={}", database.display()))
            && !logs.contains("database_id="),
        "the WARN must carry the real database path:\n{logs}"
    );
}
