//! Log-redaction tests for the manager's sync-failure records: database
//! open/verify failures and reconcile failures must carry static categories
//! and redacted ids, never the `Display` text of the underlying error (which
//! embeds absolute paths, stored roots, or ignore-style notes).

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
/// scenario). The record must carry a static kind and a redacted id.
#[test]
fn a_stale_project_root_warning_logs_no_raw_previous_root() {
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
    let expected_database_id = crate::util::redacted_path(&database);
    let expected_root_id = crate::util::redacted_path(std::path::Path::new(&stale_root));

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
        records[0].contains(&format!("database_id={expected_database_id}")),
        "the database must be named by its redacted id:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("stored_root_id={expected_root_id}")),
        "the old root must be named by its redacted id:\n{logs}"
    );
    assert!(
        !logs.contains(&stale_root) && !logs.contains("copied-from-elsewhere"),
        "the previous absolute root must not be logged:\n{logs}"
    );
    assert!(
        !logs.contains(project.path().to_string_lossy().as_ref()),
        "the current absolute root must not be logged:\n{logs}"
    );
}

/// R12: the open-failure warning used to format `StoreError`, whose
/// dependency errors can embed the database path. The record must carry a
/// static kind and a redacted id.
#[test]
fn a_database_open_failure_warning_logs_no_raw_path() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::create_dir_all(&database).expect("make the db path a directory");
    // Owner-only so the permission guard passes and the failure is the
    // SQLite open itself (`StoreError::Open`).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))
            .expect("restrict the database directory to owner-only");
    }
    let raw_database = database.to_string_lossy().to_string();
    let expected_id = crate::util::redacted_path(&database);

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
        records[0].contains("error_kind=\"open\""),
        "the record must carry the static error category:\n{logs}"
    );
    assert!(
        records[0].contains(&format!("database_id={expected_id}")),
        "the database must be named by its redacted id:\n{logs}"
    );
    assert!(
        !logs.contains(&raw_database),
        "the absolute database path must not be logged:\n{logs}"
    );
    assert!(
        !logs.contains(project.path().to_string_lossy().as_ref()),
        "the project root must not be logged:\n{logs}"
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
