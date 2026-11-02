//! `project_control` against a symlinked `project.db` (never followed) and
//! that `off` removes the database even while a reader holds a snapshot.

use super::*;
use slugaudit_test_check::{Check, TestResult};
use std::fs;

const VICTIM_BYTES: &[u8] = b"not a SlugAudit database\n";

fn request(path: &str, action: ProjectControlAction) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action,
        profile: None,
    })
}

/// A project whose `.planning/slugaudit/project.db` is a symlink to a regular
/// file outside the activation directory. Returns the project, its path, the
/// link and the target.
#[cfg(unix)]
fn project_with_symlinked_database() -> TestResult<(
    tempfile::TempDir,
    String,
    std::path::PathBuf,
    std::path::PathBuf,
)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join("src")).check("create src dir")?;
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let activation = project.path().join(".planning").join("slugaudit");
    fs::create_dir_all(&activation).check("create activation dir")?;
    let target = project.path().join("victim.db");
    fs::write(&target, VICTIM_BYTES).check("write link target")?;
    let link = activation.join("project.db");
    std::os::unix::fs::symlink(&target, &link).check("symlink project.db")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path, link, target))
}

#[cfg(unix)]
#[test]
fn a_tool_call_refuses_a_symlinked_project_db_and_leaves_the_link_and_target_alone() -> TestResult {
    let (_project, path, link, target) = project_with_symlinked_database()?;

    let error = crate::tools::report(
        &Parameters(crate::tools::ReportRequest { path }),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::new(),
    )
    .err()
    .check("a symlinked project.db is never opened")?;

    assert_eq!(
        error.message,
        "opening the project database for sync: refusing to open a database path that is a symlink"
    );
    assert!(
        fs::symlink_metadata(&link)
            .check("link still present")?
            .file_type()
            .is_symlink(),
        "the link is untouched"
    );
    assert_eq!(fs::read(&target).check("read target")?, VICTIM_BYTES);
    Ok(())
}

#[cfg(unix)]
#[test]
fn on_refuses_a_symlinked_project_db_and_leaves_the_link_and_target_alone() -> TestResult {
    let (_project, path, link, target) = project_with_symlinked_database()?;

    let error = project_control(
        &request(&path, ProjectControlAction::On),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .check("on must refuse a symlinked project.db")?;

    assert_eq!(
        error.message,
        "database: refusing to open a database path that is a symlink"
    );
    assert!(
        fs::symlink_metadata(&link)
            .check("link still present")?
            .file_type()
            .is_symlink(),
        "the link is untouched"
    );
    assert_eq!(fs::read(&target).check("read target")?, VICTIM_BYTES);
    Ok(())
}

#[cfg(unix)]
#[test]
fn off_deletes_the_directory_without_following_a_symlinked_project_db() -> TestResult {
    let (project, path, _link, target) = project_with_symlinked_database()?;

    let response = project_control(
        &request(&path, ProjectControlAction::Off),
        &crate::progress::NoopProgressSink,
        None,
    )
    .check("off deletes the directory, link included")?;

    assert_eq!(response.0.status, "disabled");
    assert!(
        !project.path().join(".planning/slugaudit").exists(),
        "the whole directory is gone"
    );
    assert_eq!(
        fs::read(&target).check("the target survives")?,
        VICTIM_BYTES,
        "off never touches what the link pointed at"
    );
    Ok(())
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

    assert!(!database.exists(), "the database is removed");
    drop(reader);
    drop(writer);
    Ok(())
}
