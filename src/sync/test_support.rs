//! Shared fixture helpers for `sync` tests. Previously each test file
//! carried its own copy (`write`: 8 copies, `stored_paths`: 3 copies,
//! `setup_project`: 2, `create_project`/`write_file`/`sync_project`: 2
//! each), so a change to how fixture files are written or queried now
//! lives in exactly one place.
use std::fs;
use std::path::Path;

use rusqlite::Connection;
use tempfile::TempDir;

use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::{SourceSyncManager, SyncedProject};

/// Writes `relative` under `root`, creating parent directories on demand.
pub(crate) fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

/// TempDir-flavored [`write`] for tests that hold a `TempDir` project.
pub(crate) fn write_file(project: &TempDir, relative: &str, content: &[u8]) {
    write(project.path(), relative, content);
}

/// Creates a tempdir with a `.planning/slugaudit` activation marker — the
/// minimum shape of an enabled project.
pub(crate) fn create_project() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::create_dir_all(dir.path().join(".planning").join("slugaudit"))
        .expect("create activation dir");
    dir
}

/// A tempdir project, a tempdir db, and a connection carrying an initial
/// publish of `a.rs`/`b.rs`. Returns the baseline revision id so the
/// reconcile suites can assert against it.
pub(crate) fn setup_project() -> (TempDir, TempDir, Connection, String) {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    // Initial publish to establish a baseline revision.
    write(project.path(), "a.rs", b"fn a() {}");
    write(project.path(), "b.rs", b"fn b() {}");
    let report = publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
        .expect("initial publish");
    (project, db_dir, connection, report.revision_id)
}

/// Runs a full `ensure_current` for `project` on `manager`.
pub(crate) fn sync_project(manager: &SourceSyncManager, project: &TempDir) -> SyncedProject {
    manager
        .ensure_current(&project.path().to_string_lossy(), &NoopProgressSink)
        .expect("sync succeeds")
}

/// Reads every stored path in the `files` table, sorted.
pub(crate) fn stored_paths(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .expect("prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}
