//! Failure-path tests for `with_verified_read` / `with_verified_write`:
//! the database-open and transaction error arms that the happy-path and
//! stale-revision tests don't reach. A handle pointed at a missing or
//! corrupt database must fail cleanly with a retry-hint message, never
//! panic and never serve stale data.

use crate::tools::context::{SyncedProject, with_verified_read, with_verified_write};
use std::path::PathBuf;

fn missing_handle() -> SyncedProject {
    SyncedProject {
        database_path: PathBuf::from("/definitely/missing/slugaudit-project.db"),
        revision_id: "rev-1".to_owned(),
        root: PathBuf::from("/definitely/missing"),
    }
}

#[test]
fn verified_read_fails_cleanly_when_the_database_is_missing() {
    let result = with_verified_read(&missing_handle(), |_tx| Ok(()));
    let error = result.expect_err("opening a missing database must fail");
    let message = error.message.to_string();
    assert!(
        message.contains("retry the call"),
        "the error must carry the retry hint, got: {message}"
    );
}

#[test]
fn verified_write_fails_cleanly_when_the_database_is_missing() {
    let result = with_verified_write(&missing_handle(), |_tx| Ok(()));
    assert!(
        result.is_err(),
        "a write against a missing database must fail cleanly, not panic"
    );
}

#[test]
fn verified_read_fails_cleanly_on_a_corrupt_database() {
    let directory = tempfile::tempdir().expect("db dir");
    let database_path = directory.path().join("project.db");
    std::fs::write(&database_path, b"this is not a sqlite database").expect("write corrupt db");
    let handle = SyncedProject {
        database_path: database_path.clone(),
        revision_id: "rev-1".to_owned(),
        root: directory.path().to_path_buf(),
    };

    let result = with_verified_read(&handle, |_tx| Ok(()));
    assert!(
        result.is_err(),
        "a corrupt database must surface an error rather than serve stale data"
    );
}
