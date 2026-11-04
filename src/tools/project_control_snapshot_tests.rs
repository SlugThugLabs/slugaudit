//! `project_control off` removes the database even while a reader holds a
//! snapshot.

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure};
use std::fs;

fn request(path: &str, action: ProjectControlAction) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action,
        profile: None,
    })
}

/// `off` removes the database even while another process holds a read
/// snapshot: the index is scratch, so no lock or checkpoint is required.
#[test]
fn off_removes_the_database_even_while_a_reader_holds_a_wal_snapshot() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join("src")).check("create src dir")?;
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::new();
    project_control(
        &request(&path, ProjectControlAction::On),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .check("on")?;
    let database = project.path().join(".planning/slugaudit/project.db");
    let writer = store::open_read_write(&database).check("open writer")?;
    writer
        .execute_batch("CREATE TABLE wal_probe(x); INSERT INTO wal_probe VALUES (1);")
        .check("leave frames in the WAL")?;
    let reader = store::open_read_write(&database).check("open reader")?;
    reader.execute_batch("BEGIN").check("begin read")?;
    let _: i64 = reader
        .query_row("SELECT count(*) FROM wal_probe", [], |row| row.get(0))
        .check("pin a read snapshot")?;

    project_control(
        &request(&path, ProjectControlAction::Off),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .check("off removes the file regardless of the reader")?;

    ensure(!database.exists(), "the database is removed")?;
    drop(reader);
    drop(writer);
    Ok(())
}
