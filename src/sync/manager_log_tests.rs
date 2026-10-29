//! Log-redaction tests for the manager's sync-failure records: database
//! open/verify failures and reconcile failures must carry static categories
//! and the real database or project path (the real-path logging policy,
//! 2026-10-29), including a stored root from another machine or location.

use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};
use std::fs;

/// The structured records for one sync-log operation, one per failure.
fn sync_failure_records<'a>(logs: &'a str, operation: &str) -> Vec<&'a str> {
    let marker = format!("operation=\"{operation}\"");
    logs.lines().filter(|line| line.contains(&marker)).collect()
}

/// R12: the stale-metadata warning used to format a reason string that
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

/// R12: the open-failure warning used to format `StoreError`, whose
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
    // Owner-only so the permission guard passes and the failure is the
    // SQLite connection setup (`StoreError::Configure`).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))
            .expect("restrict the database directory to owner-only");
    }
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

/// R12: a dirty path that cannot be read reaches the reconcile log as
/// `HashError::Read { path: <absolute> }`, which `Display` formats. The
/// record carries the static kind, the real path and the OS cause.
#[test]
fn honest_log_a_dirty_file_read_failure_logs_the_real_path_and_cause() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);
    // A regular file where a directory is required: resolving
    // `blocker/child.rs` fails with ENOTDIR, which the reconcile reports as
    // `HashError::Read`. This is deterministic even when tests run as root,
    // unlike a permission-based fixture.
    write_file(&project, "blocker", b"not a directory\n");
    let state = manager.active_watch_state().expect("registered state");
    state.mark_dirty("blocker/child.rs".to_owned());
    let absolute = project.path().join("blocker/child.rs");

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for reconcile_drain");
        manager.ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
    });

    assert!(result.is_err(), "the read failure must surface");
    assert!(
        logs.contains("capture liveness control for reconcile_drain"),
        "the capture must see this thread's events, captured: {logs}"
    );
    let records = sync_failure_records(&logs, "reconcile_drain");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("error_kind=\"hash_read\""),
        "the record must carry the static error category:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("path=Some(\"{}\")", absolute.display())),
        "the file must be named by its real path:\n{logs}"
    );
    assert!(
        records[0].contains("Not a directory"),
        "the OS cause must be logged:\n{logs}"
    );
}

/// `ReconcileError::TimeBudgetExceeded` embeds a pre-formatted note naming
/// the path being processed. The record carries the static kind, the real
/// root and the note.
#[test]
fn honest_log_a_reconcile_budget_failure_logs_the_root_and_note() {
    let root = std::path::Path::new("/home/private-user/secret-repo");
    let note = "while processing src/private-name.rs";
    let error =
        super::SyncError::Reconcile(crate::sync::reconcile::ReconcileError::TimeBudgetExceeded {
            path: format!(" {note}"),
            elapsed_ms: 42,
        });

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for reconcile_budget");
        crate::sync::manager_health_support::log_sync_failure(
            "reconcile_budget",
            root,
            "incremental reconcile failed; marking watcher Desynced so next call re-verifies",
            &error,
        );
    });

    assert!(
        logs.contains("capture liveness control for reconcile_budget"),
        "the capture must see this thread's events, captured: {logs}"
    );
    let records = sync_failure_records(&logs, "reconcile_budget");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("error_kind=\"reconcile_budget_exceeded\""),
        "the record must carry the static error category:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("path=Some(\"{note}\")")),
        "the pending file note must be logged:\n{logs}"
    );
    assert!(
        records[0].contains("root=/home/private-user/secret-repo"),
        "the real root must be logged:\n{logs}"
    );
}

/// An `OpenCorrupt` recovery record: the real database path, the source
/// error text and the unknown-count error all reach the WARN.
#[test]
fn an_open_corrupt_recovery_warn_logs_real_path_and_source_error() {
    use crate::sync::recovery_log::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let error = crate::store::StoreError::Symlink;
    let (discard, count_error) = crate::sync::DatabaseDiscard::assess(
        Err(rusqlite::Error::InvalidQuery),
        "corrupt".to_owned(),
    );

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(
            database,
            DatabaseRecovery::OpenCorrupt {
                error: &error,
                discard: &discard,
                count_error: count_error.as_ref(),
            },
        );
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
        records[0].contains("findings_discarded=None")
            && records[0].contains(&format!(
                "count_error=\"{}\"",
                rusqlite::Error::InvalidQuery
            )),
        "the record must carry the unknown count and its error:\n{logs}"
    );
    assert!(
        !records[0].contains("database_id="),
        "the redacted database id is gone:\n{logs}"
    );
}

/// An `IncompatibleVersion` recovery record carries the real database path,
/// both versions and the discard count.
#[test]
fn an_incompatible_version_recovery_warn_logs_real_path_and_versions() {
    use crate::sync::recovery_log::{DatabaseRecovery, log_database_recovery};
    let database = std::path::Path::new("/home/private-user/secret-repo/project.db");
    let (discard, count_error) = crate::sync::DatabaseDiscard::assess(Ok(3), "old".to_owned());

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        log_database_recovery(
            database,
            DatabaseRecovery::IncompatibleVersion {
                contract_version: 7,
                schema_version: 9,
                discard: &discard,
                count_error: count_error.as_ref(),
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
            && records[0].contains("schema_version=9")
            && records[0].contains("findings_discarded=Some(3)"),
        "the record must carry the category, versions and count:\n{logs}"
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
        crate::sync::manager_health_support::revision_after_sync(&connection, &database)
    });

    assert!(result.is_err(), "no revision must surface as an error");
    assert!(
        logs.contains("no current revision found after sync")
            && logs.contains(&format!("path={}", database.display()))
            && !logs.contains("database_id="),
        "the WARN must carry the real database path:\n{logs}"
    );
}
