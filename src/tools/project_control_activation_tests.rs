//! The on/off switch is the directory `.planning/slugaudit/`: `on` creates it,
//! `off` deletes it, a missing or broken `project.db` inside it is rebuilt by
//! the next tool call, and no tool call ever creates the directory.

use super::*;
use std::fs;

fn project_with_source() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

fn control(path: &str, action: ProjectControlAction) -> ProjectControlResponse {
    project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path.to_owned()),
            action,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("project_control succeeds")
    .0
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

fn assert_not_enabled(error: &ErrorData) {
    assert_eq!(
        error.message,
        "SlugAudit is not enabled for this project or any parent directory"
    );
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!("project_not_enabled")),
        "{error:?}"
    );
}

#[test]
fn on_creates_the_directory_the_database_and_the_gitignore_entry_and_a_call_works() {
    let (project, path) = project_with_source();
    let activation = project.path().join(".planning/slugaudit");

    let on = control(&path, ProjectControlAction::On);

    assert_eq!(on.status, "enabled");
    assert!(activation.is_dir());
    assert!(activation.join("project.db").is_file());
    let gitignore = fs::read_to_string(project.path().join(".gitignore")).expect("gitignore");
    assert!(gitignore.contains(".planning/slugaudit"), "{gitignore}");
    let answer = report(&path, &sync::SourceSyncManager::new()).expect("a call after on works");
    assert_eq!(answer.file_count, 2, "lib.rs and .gitignore are indexed");
}

#[test]
fn off_deletes_the_directory_with_config_and_extra_files_then_an_empty_planning_dir() {
    let (project, path) = project_with_source();
    control(&path, ProjectControlAction::On);
    let activation = project.path().join(".planning/slugaudit");
    fs::write(activation.join("config.json"), b"{}").expect("write config");
    fs::write(activation.join("extra.txt"), b"x").expect("write extra");

    let off = control(&path, ProjectControlAction::Off);

    assert_eq!(off.status, "disabled");
    assert_eq!(off.removed.len(), 2, "{:?}", off.removed);
    assert!(!project.path().join(".planning").exists());
}

#[test]
fn off_keeps_a_planning_dir_that_holds_other_files() {
    let (project, path) = project_with_source();
    control(&path, ProjectControlAction::On);
    let planning = project.path().join(".planning");
    fs::write(planning.join("PLAN.md"), b"mine").expect("write user file");

    let off = control(&path, ProjectControlAction::Off);

    assert_eq!(off.removed.len(), 1, "{:?}", off.removed);
    assert!(!planning.join("slugaudit").exists());
    assert_eq!(fs::read(planning.join("PLAN.md")).expect("kept"), b"mine");
}

#[test]
fn a_deleted_project_db_is_rebuilt_by_the_next_call() {
    let (project, path) = project_with_source();
    control(&path, ProjectControlAction::On);
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::remove_file(&database).expect("delete project.db");

    let answer = report(&path, &sync::SourceSyncManager::new()).expect("the call rebuilds");

    assert!(database.is_file(), "project.db is back");
    assert_eq!(answer.file_count, 2);
    assert!(answer.database_discard.is_none(), "{answer:?}");
}

#[test]
fn a_garbage_project_db_is_rebuilt_by_the_next_call() {
    let (project, path) = project_with_source();
    control(&path, ProjectControlAction::On);
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, vec![0xAB_u8; 8192]).expect("write garbage");

    let answer = report(&path, &sync::SourceSyncManager::new()).expect("the call rebuilds");

    assert_eq!(answer.file_count, 2);
    assert!(answer.database_discard.is_some(), "the rebuild is reported");
    store::open_existing(&database).expect("the rebuilt database opens");
}

#[test]
fn a_call_on_a_repo_without_the_directory_is_not_enabled_and_creates_nothing() {
    let (project, path) = project_with_source();

    let error = report(&path, &sync::SourceSyncManager::new()).expect_err("not enabled");

    assert_not_enabled(&error);
    assert!(!project.path().join(".planning/slugaudit").exists());
    assert!(!project.path().join(".planning").exists());
}

#[test]
fn a_directory_removed_between_two_calls_makes_the_second_not_enabled() {
    let (project, path) = project_with_source();
    control(&path, ProjectControlAction::On);
    let manager = sync::SourceSyncManager::new();
    report(&path, &manager).expect("first call works");
    fs::remove_dir_all(project.path().join(".planning/slugaudit")).expect("remove directory");

    let error = report(&path, &manager).expect_err("second call is not enabled");

    assert_not_enabled(&error);
    assert!(!project.path().join(".planning/slugaudit").exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_activation_directory_is_off_and_off_removes_only_the_link() {
    let (project, path) = project_with_source();
    let target = project.path().join("elsewhere");
    fs::create_dir_all(&target).expect("create target");
    fs::write(target.join("keep.txt"), b"keep").expect("write target file");
    fs::create_dir_all(project.path().join(".planning")).expect("planning");
    let link = project.path().join(".planning/slugaudit");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    let error = report(&path, &sync::SourceSyncManager::new()).expect_err("a symlink is off");
    assert_not_enabled(&error);

    let off = control(&path, ProjectControlAction::Off);

    assert_eq!(off.status, "disabled");
    assert!(link.symlink_metadata().is_err(), "the link is gone");
    assert_eq!(fs::read(target.join("keep.txt")).expect("kept"), b"keep");
}
