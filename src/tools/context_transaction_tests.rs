//! Failure-path tests for `with_verified_read` / `with_verified_write`:
//! the database-open and transaction error arms that the happy-path and
//! stale-revision tests don't reach. A handle pointed at a missing or
//! corrupt database must fail cleanly with a retry-hint message, never
//! panic and never serve stale data.

use crate::tools::context::{
    SyncedProject, ensure_synced_no_progress, with_verified_read, with_verified_write,
};
use crate::tools::test_support::activated_project;
use std::path::PathBuf;

fn missing_handle() -> SyncedProject {
    SyncedProject {
        database_path: PathBuf::from("/definitely/missing/slugaudit-project.db"),
        revision_id: "rev-1".to_owned(),
    }
}

fn synced_project() -> (tempfile::TempDir, SyncedProject) {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced_no_progress(
        &project.path().to_string_lossy(),
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("sync");
    (project, synced)
}

#[test]
fn verified_read_fails_cleanly_when_the_database_is_missing() {
    let result = with_verified_read(&missing_handle(), |_tx| Ok(()));
    let error = result.expect_err("opening a missing database must fail");
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("opening the project database:"),
        "the error must name the failed open, got: {}",
        error.message
    );
    assert!(
        error.message.contains("retry the call"),
        "the error must carry the retry hint, got: {}",
        error.message
    );
}

#[test]
fn verified_write_fails_cleanly_when_the_database_is_missing() {
    let result = with_verified_write(&missing_handle(), |_tx| Ok(()));
    let error =
        result.expect_err("a write against a missing database must fail cleanly, not panic");
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error
            .message
            .starts_with("opening the project database for write:"),
        "the error must name the failed write open, got: {}",
        error.message
    );
    assert!(
        error.message.contains("retry the call"),
        "the error must carry the retry hint, got: {}",
        error.message
    );
}

#[test]
fn verified_read_fails_cleanly_on_a_corrupt_database() {
    let directory = tempfile::tempdir().expect("db dir");
    let database_path = directory.path().join("project.db");
    std::fs::write(&database_path, b"this is not a sqlite database").expect("write corrupt db");
    let handle = SyncedProject {
        database_path,
        revision_id: "rev-1".to_owned(),
    };

    let result = with_verified_read(&handle, |_tx| Ok(()));
    assert!(
        result.is_err(),
        "a corrupt database must surface an error rather than serve stale data"
    );
}

/// A database that opens but has no `revisions` table forces the SELECT in
/// `verify_revision_matches` to fail, the arm the stale-revision test cannot
/// reach: the handle is current, but the read itself breaks.
#[test]
fn verified_read_reports_a_failure_reading_the_current_revision() {
    let directory = tempfile::tempdir().expect("db dir");
    let database_path = directory.path().join("project.db");
    drop(rusqlite::Connection::open(&database_path).expect("create empty database"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&database_path, std::fs::Permissions::from_mode(0o600))
            .expect("restrict database permissions");
    }
    let handle = SyncedProject {
        database_path,
        revision_id: "rev-1".to_owned(),
    };

    let error = with_verified_read(&handle, |_tx| Ok(()))
        .expect_err("a database without a revisions table must fail the read");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("reading the current revision:"),
        "the error must name the failed read, got: {}",
        error.message
    );
    assert!(
        error.message.contains("no such table: revisions"),
        "the SQLite cause must be kept, got: {}",
        error.message
    );
}

/// `verify_revision_matches` passes, the closure runs, but the transaction is
/// no longer open when the wrapper commits: the commit arm must translate the
/// failure with its own context instead of panicking.
#[test]
fn verified_read_reports_a_failed_commit() {
    let (_project, synced) = synced_project();

    let error = with_verified_read(&synced, |tx| {
        tx.execute_batch("COMMIT")
            .expect("end the transaction inside the closure");
        Ok(())
    })
    .expect_err("committing an already-ended transaction must fail");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error
            .message
            .starts_with("committing the read transaction:"),
        "the error must name the failed commit, got: {}",
        error.message
    );
    assert!(
        error.message.contains("no transaction is active"),
        "the SQLite cause must be kept, got: {}",
        error.message
    );
}

#[test]
fn verified_write_reports_a_failed_commit() {
    let (_project, synced) = synced_project();

    let error = with_verified_write(&synced, |tx| {
        tx.execute_batch("COMMIT")
            .expect("end the transaction inside the closure");
        Ok(())
    })
    .expect_err("committing an already-ended transaction must fail");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error
            .message
            .starts_with("committing the write transaction:"),
        "the error must name the failed commit, got: {}",
        error.message
    );
    assert!(
        error.message.contains("no transaction is active"),
        "the SQLite cause must be kept, got: {}",
        error.message
    );
}

/// `BEGIN DEFERRED` on a connection that already has an open transaction is
/// the one reachable start failure: the wrapper must translate SQLite's
/// refusal with the start context and keep the cause, not panic.
#[test]
fn verified_read_reports_a_failed_transaction_start_with_an_active_transaction() {
    let mut connection = rusqlite::Connection::open_in_memory().expect("open connection");
    connection
        .execute_batch("BEGIN DEFERRED")
        .expect("open the outer transaction");
    let cause = connection
        .execute_batch("BEGIN DEFERRED")
        .expect_err("SQLite refuses a second BEGIN");

    let error = super::context_transactions::start_read_transaction(&mut connection)
        .expect_err("starting a nested read transaction must fail");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(
        error.message,
        format!(
            "starting a read transaction: {cause}. This is often transient (a concurrent sync, \
             publish, or disable) — retry the call. If it keeps happening, the project may need \
             to be disabled and re-enabled."
        )
    );
    connection
        .execute_batch("ROLLBACK")
        .expect("close the outer transaction");
}

/// Another connection holds the database's write lock, so `BEGIN IMMEDIATE`
/// exhausts the busy timeout and the start arm translates the failure.
#[test]
fn verified_write_reports_a_failed_transaction_start_when_the_write_lock_is_held() {
    let (_project, synced) = synced_project();
    let blocker = rusqlite::Connection::open(&synced.database_path).expect("open blocker");
    blocker
        .busy_timeout(std::time::Duration::ZERO)
        .expect("disable the blocker's busy timeout");
    blocker
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold the write lock");

    let error = with_verified_write(&synced, |_tx| Ok(()))
        .expect_err("a second writer cannot start an immediate transaction");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("starting a write transaction:"),
        "the error must name the failed start, got: {}",
        error.message
    );
    assert!(
        error.message.contains("database is locked"),
        "the SQLite cause must be kept, got: {}",
        error.message
    );
    blocker
        .execute_batch("ROLLBACK")
        .expect("release the write lock");
}
