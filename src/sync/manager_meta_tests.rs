//! Tests for the metadata helpers split out of `manager.rs`.

use super::{current_revision_id, ensure_project_row};
use crate::store;
use std::path::Path;
use tempfile::TempDir;

fn open_temp_db() -> (TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("project.db");
    let connection = store::open_read_write(&db_path).expect("open db");
    (dir, connection)
}

#[test]
fn current_revision_id_returns_none_when_no_revision_has_been_published() {
    let (_dir, connection) = open_temp_db();
    let revision = current_revision_id(&connection).expect("query");
    assert!(revision.is_none());
}

#[test]
fn ensure_project_row_succeeds_on_a_fresh_database() {
    let (_dir, mut connection) = open_temp_db();
    let root = Path::new("/tmp/example-project");
    ensure_project_row(&mut connection, root).expect("first ensure");
    // Calling twice must be idempotent (the row already exists).
    ensure_project_row(&mut connection, root).expect("idempotent on second call");
}

#[test]
fn ensure_project_row_detects_a_mismatched_root_via_invalid_params() {
    let (_dir, mut connection) = open_temp_db();
    let first_root = Path::new("/tmp/first-root");
    ensure_project_row(&mut connection, first_root).expect("first ensure");
    let second_root = Path::new("/tmp/different-root");
    let err = ensure_project_row(&mut connection, second_root).expect_err("must error");
    // The implementation uses invalid_params for a root-mismatch since
    // the caller can recover by disabling and re-enabling.
    assert!(
        err.message.contains("different project root")
            || err.message.contains("belongs to a different"),
        "got: {err:?}",
    );
}
