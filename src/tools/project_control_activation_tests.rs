//! The on/off switch is the directory `.planning/slugaudit/`: `on` creates it,
//! `off` deletes it, a missing or broken `project.db` inside it is rebuilt by
//! the next tool call, and no tool call ever creates the directory.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

fn project_with_source() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

fn control(path: &str, action: ProjectControlAction) -> TestResult<ProjectControlResponse> {
    Ok(project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path.to_owned()),
            action,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        None,
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
        "error.message == \"SlugAudit is not enabled for this project or any parent directory\"",
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
    ensure_eq(&answer.file_count, &2, "lib.rs and .gitignore are indexed")?;
    Ok(())
}

#[test]
fn off_deletes_the_directory_with_config_and_extra_files_then_an_empty_planning_dir() -> TestResult
{
    let (project, path) = project_with_source()?;
    control(&path, ProjectControlAction::On)?;
    let activation = project.path().join(".planning/slugaudit");
    fs::write(activation.join("config.json"), b"{}").check("write config")?;
    fs::write(activation.join("extra.txt"), b"x").check("write extra")?;

    let off = control(&path, ProjectControlAction::Off)?;

    ensure_eq(
        &off.status,
        &ProjectControlStatus::Disabled,
        "off.status == ProjectControlStatus::Disabled",
    )?;
    ensure_eq(&off.removed.len(), &2, format!("{:?}", off.removed))?;
    ensure(
        !project.path().join(".planning").exists(),
        "!project.path().join(\".planning\").exists()",
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
        "fs::read(planning.join(\"PLAN.md\")).check(\"kept\")? == b\"mine\"",
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
    ensure_eq(&answer.file_count, &2, "answer.file_count == 2")?;
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

    ensure_eq(&answer.file_count, &2, "answer.file_count == 2")?;
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
