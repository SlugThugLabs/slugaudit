//! The profile `project_control on` takes is a session setting, and `off`
//! leaves nothing behind.
use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

fn on_with_profile(path: &str, profile: &str) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action: ProjectControlAction::On,
        profile: Some(profile.to_owned()),
    })
}

fn activation_entries(project: &tempfile::TempDir) -> TestResult<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(project.path().join(".planning/slugaudit"))
        .check("activation dir")?
        .map(|entry| {
            entry
                .check("directory entry")
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<TestResult<_>>()?;
    names.sort();
    Ok(names)
}

#[test]
fn turning_on_with_a_profile_holds_it_in_memory_and_writes_no_config() -> TestResult {
    let (project, path) = indexed_project()?;
    let manager = sync::SourceSyncManager::new();

    project_control(
        &on_with_profile(&path, "lean"),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("enable with a profile")?;

    let root = ProjectRoot::resolve(project.path()).check("resolve root")?;
    ensure_eq(
        &project::session_profile(root.as_path()).check("read the session profile")?,
        &Some(crate::model::AuditProfile::Lean),
        "session profile stored in memory",
    )?;
    let entries = activation_entries(&project)?;
    ensure(
        !entries.iter().any(|name| name.starts_with("config.")),
        format!("no config file is written: {entries:?}"),
    )?;
    let limits = project::effective_limits(root.as_path()).check("limits")?;
    ensure_eq(
        &limits.max_total_import_bytes,
        &crate::model::limits_for_profile(crate::model::AuditProfile::Lean)
            .check("lean")?
            .max_total_import_bytes,
        "the session profile picks the base limits",
    )?;
    Ok(())
}

#[test]
fn a_per_repo_settings_file_in_activation_dir_is_ignored_when_turning_on() -> TestResult {
    let (project, path) = indexed_project()?;
    let activation = project.path().join(".planning/slugaudit");
    std::fs::create_dir_all(&activation).check("activation dir")?;
    let legacy_name = ["config", ".json"].concat();
    std::fs::write(
        activation.join(legacy_name),
        br#"{"max_total_import_bytes": 1}"#,
    )
    .check("an old install's per-repo file")?;

    let response = control(&path, ProjectControlAction::On)?.0;

    ensure_eq(
        &response.status,
        &ProjectControlStatus::Enabled,
        "response.status == ProjectControlStatus::Enabled",
    )?;
    response
        .import
        .as_ref()
        .check("the 1-byte ceiling in the ignored file was not applied")?;
    Ok(())
}

fn control(path: &str, action: ProjectControlAction) -> TestResult<Json<ProjectControlResponse>> {
    let manager = sync::SourceSyncManager::new();
    project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path.to_owned()),
            action,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("project_control succeeds")
}

fn indexed_project() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

#[test]
fn a_tool_call_after_project_off_is_not_active_and_does_not_recreate_the_directory() -> TestResult {
    let (project, path) = indexed_project()?;
    control(&path, ProjectControlAction::On)?;
    let activation = project.path().join(".planning/slugaudit");
    std::fs::write(activation.join("extra.json"), b"{}").check("write extra")?;

    let off = control(&path, ProjectControlAction::Off)?.0;

    ensure_eq(
        &off.status,
        &ProjectControlStatus::Disabled,
        "off.status == ProjectControlStatus::Disabled",
    )?;
    ensure(!activation.exists(), "off deletes the whole directory")?;
    let error = crate::tools::report(
        &Parameters(crate::tools::ReportRequest { path: path.clone() }),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::new(),
    )
    .err()
    .check("a call on a project that is off fails")?;
    ensure_eq(
        &error.data.as_ref().and_then(|data| data.get("code")),
        &Some(&serde_json::json!("project_not_enabled")),
        format!("{error:?}"),
    )?;
    ensure(
        !activation.exists(),
        "the failed call must not recreate the directory",
    )?;
    ensure_eq(
        &control(&path, ProjectControlAction::Off)?.0.status,
        &ProjectControlStatus::AlreadyDisabled,
        "a second off has nothing left to remove",
    )?;
    Ok(())
}

#[test]
fn off_fails_when_the_activation_dir_cannot_be_inspected() -> TestResult {
    let (project, path) = indexed_project()?;
    let manager = sync::SourceSyncManager::new();
    std::fs::write(project.path().join(".planning"), b"x").check("write file")?;

    let error = project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path),
            action: ProjectControlAction::Off,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("an uninspectable activation dir is not 'already disabled'")?;

    ensure_eq(
        &error.message,
        &"disable: failed to update the activation directory: Not a directory (os error 20)",
        "an uninspectable activation dir fails the operation with its cause",
    )?;
    Ok(())
}
