//! Activation directory lifecycle, WAL snapshot cleanup, and recovery log tests
//! for `project_control`.

use super::*;
use crate::store;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

fn project_with_source() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

fn control(path: &str, action: ProjectControlAction) -> TestResult<ProjectControlResponse> {
    let manager = sync::SourceSyncManager::new();
    Ok(project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path.to_owned()),
            action,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("project_control succeeds")?
    .0)
}

fn report(
    path: &str,
    manager: &sync::SourceSyncManager,
) -> Result<crate::tools::ReportResponse, ErrorData> {
    crate::tools::report(
        &Parameters(crate::tools::ReportRequest {
            path: path.to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|response| response.0)
}

fn assert_not_enabled(error: &ErrorData) -> TestResult {
    ensure_eq(
        &error.message,
        &"SlugAudit is not enabled for this project or any parent directory",
        "not-enabled message matches contract",
    )?;
    ensure_eq(
        &error.data.as_ref().and_then(|data| data.get("code")),
        &Some(&serde_json::json!("project_not_enabled")),
        format!("{error:?}"),
    )?;
    Ok(())
}

#[test]
fn on_creates_the_directory_the_database_and_the_gitignore_entry_and_a_call_works() -> TestResult {
    let (project, path) = project_with_source()?;
    let activation = project.path().join(".planning/slugaudit");

    let on = control(&path, ProjectControlAction::On)?;

    ensure_eq(
        &on.status,
        &ProjectControlStatus::Enabled,
        "on.status == ProjectControlStatus::Enabled",
    )?;
    ensure(activation.is_dir(), "activation.is_dir()")?;
    ensure(
        activation.join("project.db").is_file(),
        "activation.join(\"project.db\").is_file()",
    )?;
    let gitignore = fs::read_to_string(project.path().join(".gitignore")).check("gitignore")?;
    ensure(gitignore.contains(".planning/slugaudit"), &gitignore)?;
    let answer = report(&path, &sync::SourceSyncManager::new()).check("a call after on works")?;
    ensure_eq(
        &answer.file_count,
        &2,
        "lib.rs and .gitignore are indexed (.planning/ is excluded)",
    )?;
    Ok(())
}

#[test]
fn off_deletes_the_directory_with_extra_files_and_keeps_the_planning_readme() -> TestResult {
    let (project, path) = project_with_source()?;
    control(&path, ProjectControlAction::On)?;
    let activation = project.path().join(".planning/slugaudit");
    fs::write(activation.join("extra.txt"), b"x").check("write extra")?;

    let off = control(&path, ProjectControlAction::Off)?;

    ensure_eq(
        &off.status,
        &ProjectControlStatus::Disabled,
        "off.status == ProjectControlStatus::Disabled",
    )?;
    ensure_eq(
        &off.removed.len(),
        &1,
        format!("off removes only SlugAudit's own folder: {:?}", off.removed),
    )?;
    ensure(
        project.path().join(".planning/README.md").is_file(),
        "the shared .planning README belongs to every slug tool and stays",
    )?;
    Ok(())
}

#[test]
fn off_keeps_a_planning_dir_that_holds_other_files() -> TestResult {
    let (project, path) = project_with_source()?;
    control(&path, ProjectControlAction::On)?;
    let planning = project.path().join(".planning");
    fs::write(planning.join("PLAN.md"), b"mine").check("write user file")?;

    let off = control(&path, ProjectControlAction::Off)?;

    ensure_eq(&off.removed.len(), &1, format!("{:?}", off.removed))?;
    ensure(
        !planning.join("slugaudit").exists(),
        "!planning.join(\"slugaudit\").exists()",
    )?;
    ensure_eq(
        &fs::read(planning.join("PLAN.md")).check("kept")?,
        &b"mine",
        "PLAN.md kept untouched",
    )?;
    Ok(())
}

#[test]
fn a_deleted_project_db_is_rebuilt_by_the_next_call() -> TestResult {
    let (project, path) = project_with_source()?;
    control(&path, ProjectControlAction::On)?;
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::remove_file(&database).check("delete project.db")?;

    let answer = report(&path, &sync::SourceSyncManager::new()).check("the call rebuilds")?;

    ensure(database.is_file(), "project.db is back")?;
    ensure_eq(
        &answer.file_count,
        &2,
        "lib.rs and .gitignore are indexed again (.planning/ is excluded)",
    )?;
    ensure(answer.database_discard.is_none(), format!("{answer:?}"))?;
    Ok(())
}

#[test]
fn a_garbage_project_db_is_rebuilt_by_the_next_call() -> TestResult {
    let (project, path) = project_with_source()?;
    control(&path, ProjectControlAction::On)?;
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, vec![0xAB_u8; 8192]).check("write garbage")?;

    let answer = report(&path, &sync::SourceSyncManager::new()).check("the call rebuilds")?;

    ensure_eq(
        &answer.file_count,
        &2,
        "lib.rs and .gitignore are indexed again (.planning/ is excluded)",
    )?;
    answer
        .database_discard
        .as_ref()
        .check("the rebuild is reported")?;
    store::open_existing(&database).check("the rebuilt database opens")?;
    Ok(())
}

#[test]
fn a_call_on_a_repo_without_the_directory_is_not_enabled_and_creates_nothing() -> TestResult {
    let (project, path) = project_with_source()?;

    let error = report(&path, &sync::SourceSyncManager::new()).check_err("not enabled")?;

    assert_not_enabled(&error)?;
    ensure(
        !project.path().join(".planning/slugaudit").exists(),
        "!project.path().join(\".planning/slugaudit\").exists()",
    )?;
    ensure(
        !project.path().join(".planning").exists(),
        "!project.path().join(\".planning\").exists()",
    )?;
    Ok(())
}

#[test]
fn a_directory_removed_between_two_calls_makes_the_second_not_enabled() -> TestResult {
    let (project, path) = project_with_source()?;
    control(&path, ProjectControlAction::On)?;
    let manager = sync::SourceSyncManager::new();
    report(&path, &manager).check("first call works")?;
    fs::remove_dir_all(project.path().join(".planning/slugaudit")).check("remove directory")?;

    let error = report(&path, &manager).check_err("second call is not enabled")?;

    assert_not_enabled(&error)?;
    ensure(
        !project.path().join(".planning/slugaudit").exists(),
        "!project.path().join(\".planning/slugaudit\").exists()",
    )?;
    Ok(())
}

#[test]
fn off_removes_the_database_even_while_a_reader_holds_a_wal_snapshot() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join("src")).check("create src dir")?;
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::new();
    project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path.clone()),
            action: ProjectControlAction::On,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
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
        &Parameters(ProjectControlRequest {
            path: Some(path),
            action: ProjectControlAction::Off,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("off removes the file regardless of the reader")?;

    ensure(!database.exists(), "the database is removed")?;
    drop(reader);
    drop(writer);
    Ok(())
}

#[test]
fn on_action_corrupt_database_discard_warn_logs_the_real_path() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::write(project.path().join("lib.rs"), "pub fn a() {}\n").check("source file")?;
    let path = project.path().to_string_lossy().into_owned();
    let activation = project.path().join(".planning").join("slugaudit");
    let database = activation.join("project.db");
    fs::create_dir_all(&activation).check("db dir")?;
    fs::write(&database, vec![0xA5_u8; 4096]).check("write a corrupt database")?;
    let expected_path = database
        .canonicalize()
        .check("canonical database path")?
        .display()
        .to_string();
    let manager = sync::SourceSyncManager::new();

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for enable_project_discard");
        project_control(
            &Parameters(ProjectControlRequest {
                action: ProjectControlAction::On,
                path: Some(path.clone()),
                profile: None,
            }),
            &crate::progress::NoopProgressSink,
            &manager,
        )
    })?;

    let response = result.check("the corrupt database must be discarded and rebuilt")?;
    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::Enabled,
        "response.0.status == ProjectControlStatus::Enabled",
    )?;
    let record = logs
        .lines()
        .find(|line| line.contains("operation=\"open_project_database\""))
        .check(&format!("the discard record must exist:\n{logs}"))?;
    ensure(
        record.contains(&format!(" path={expected_path} ")),
        format!("the discard WARN must carry the real database path:\n{logs}"),
    )?;
    ensure(
        record
            .contains("database is corrupt or outdated; discarding and re-publishing from scratch"),
        format!("the record must be the discard WARN:\n{logs}"),
    )?;
    ensure(
        !record.contains("database_id="),
        format!("the redacted database id is gone:\n{logs}"),
    )?;
    Ok(())
}
