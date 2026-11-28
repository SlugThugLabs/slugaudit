//! Shared fixture helpers for `sync` tests, so a change to how fixture files
//! are written or queried lives in exactly one place.
use slugaudit_test_check::{Check, TestResult};
use std::fs;
use std::path::Path;

use rusqlite::{Connection, Transaction, params};
use tempfile::TempDir;

use super::FileRecord;
use super::revision_edge_invalidation::{EdgeChangeScope, is_cargo_toml_path};
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::{SourceSyncManager, SyncedProject};

/// Writes `relative` under `root`, creating parent directories on demand.
pub(crate) fn write(root: &Path, relative: &str, content: &[u8]) -> TestResult {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).check("create parent dirs")?;
    }
    fs::write(path, content).check("write fixture file")?;
    Ok(())
}

/// TempDir-flavored [`write`] for tests that hold a `TempDir` project.
pub(crate) fn write_file(project: &TempDir, relative: &str, content: &[u8]) -> TestResult {
    write(project.path(), relative, content)?;
    Ok(())
}

/// Creates a tempdir with a `.planning/slugaudit` activation directory — the
/// minimum shape of an enabled project.
pub(crate) fn create_project() -> TestResult<TempDir> {
    let dir = tempfile::tempdir().check("create temp dir")?;
    fs::create_dir_all(dir.path().join(".planning").join("slugaudit"))
        .check("create activation dir")?;
    crate::project::write_database_marker(&dir.path().join(".planning").join("slugaudit"))?;
    Ok(dir)
}

/// A tempdir project, a tempdir db, and a connection carrying an initial
/// publish of `a.rs`/`b.rs`. Returns the baseline revision id so the
/// publish suites can assert against it.
pub(crate) fn setup_project() -> TestResult<(TempDir, TempDir, Connection, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    // Initial publish to establish a baseline revision.
    write(project.path(), "a.rs", b"fn a() {}")?;
    write(project.path(), "b.rs", b"fn b() {}")?;
    let report = publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
        .check("initial publish")?;
    Ok((project, db_dir, connection, report.revision_id))
}

/// Runs a full `ensure_current` for `project` on `manager`.
pub(crate) fn sync_project(
    manager: &SourceSyncManager,
    project: &TempDir,
) -> TestResult<SyncedProject> {
    manager
        .ensure_current(&project.path().to_string_lossy(), &NoopProgressSink)
        .check("sync succeeds")
}

/// Returns a generous sync/parse deadline for non-timeout tests.
pub(crate) fn generous_deadline() -> crate::util::Deadline {
    crate::util::Deadline::after(std::time::Duration::from_mins(10))
}

/// Reads every stored path in the `files` table, sorted.
pub(crate) fn stored_paths(connection: &Connection) -> TestResult<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .check("prepare")?;
    statement
        .query_map([], |row| row.get(0))
        .check("query")?
        .collect::<Result<_, _>>()
        .check("collect")
}

/// Derives [`EdgeChangeScope`] for `publish_revision` test inputs by checking
/// which `upserts` are newly added vs already stored in `files`.
pub(super) fn scope_for_test_inputs(
    tx: &Transaction<'_>,
    upserts: &[FileRecord],
    deletions: &[String],
    re_resolve_unresolved: bool,
) -> Result<EdgeChangeScope, rusqlite::Error> {
    let mut scope = EdgeChangeScope {
        cargo_toml_touched: upserts.iter().any(|f| is_cargo_toml_path(&f.relative_path))
            || deletions.iter().any(|p| is_cargo_toml_path(p)),
        ..EdgeChangeScope::default()
    };
    if !re_resolve_unresolved {
        return Ok(scope);
    }
    let mut exists_stmt =
        tx.prepare_cached("SELECT EXISTS(SELECT 1 FROM files WHERE path = ?1)")?;
    for file in upserts {
        let already_exists: bool =
            exists_stmt.query_row(params![file.relative_path], |row| row.get(0))?;
        if !already_exists {
            scope.record_path(&file.relative_path, true);
        }
    }
    for path in deletions {
        scope.record_path(path, false);
    }
    Ok(scope)
}
