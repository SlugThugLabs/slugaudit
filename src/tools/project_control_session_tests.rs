//! The profile `project_control on` takes is a session setting, and `off`
//! leaves nothing behind.
use super::*;

fn on_with_profile(path: &str, profile: &str) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action: ProjectControlAction::On,
        profile: Some(profile.to_owned()),
    })
}

/// The names in the activation directory, sorted.
fn activation_entries(project: &tempfile::TempDir) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(project.path().join(".planning/slugaudit"))
        .expect("activation dir")
        .map(|entry| {
            entry
                .expect("directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn turning_on_with_a_profile_holds_it_in_memory_and_writes_no_config() {
    let (project, path) = indexed_project();

    project_control(
        &on_with_profile(&path, "lean"),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("enable with a profile");

    let root = ProjectRoot::resolve(project.path()).expect("resolve root");
    assert_eq!(
        project::session_profile(root.as_path()),
        Some(crate::model::AuditProfile::Lean)
    );
    let entries = activation_entries(&project);
    assert!(
        !entries.iter().any(|name| name.starts_with("config.")),
        "no config file is written: {entries:?}"
    );
    let limits = project::effective_limits(root.as_path()).expect("limits");
    assert_eq!(
        limits.max_total_import_bytes,
        crate::model::limits_for_profile(crate::model::AuditProfile::Lean)
            .expect("lean")
            .max_total_import_bytes,
        "the session profile picks the base limits"
    );
}

#[test]
fn a_per_repo_config_json_is_ignored_when_turning_on() {
    let (project, path) = indexed_project();
    let activation = project.path().join(".planning/slugaudit");
    std::fs::create_dir_all(&activation).expect("activation dir");
    std::fs::write(
        activation.join("config.json"),
        br#"{"max_total_import_bytes": 1}"#,
    )
    .expect("an old install's config");

    let response = control(&path, ProjectControlAction::On).0;

    assert_eq!(response.status, "enabled");
    assert!(
        response.import.is_some(),
        "the 1-byte ceiling in the old file was not applied"
    );
}

fn control(path: &str, action: ProjectControlAction) -> Json<ProjectControlResponse> {
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
}

fn indexed_project() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

#[test]
fn a_tool_call_after_project_off_is_not_active_and_does_not_recreate_the_directory() {
    let (project, path) = indexed_project();
    control(&path, ProjectControlAction::On);
    let activation = project.path().join(".planning/slugaudit");
    std::fs::write(activation.join("config.json"), b"{}").expect("write config");

    let off = control(&path, ProjectControlAction::Off).0;

    assert_eq!(off.status, "disabled");
    assert!(!activation.exists(), "off deletes the whole directory");
    assert!(!project.path().join(".planning").exists());
    let error = crate::tools::report(
        &Parameters(crate::tools::ReportRequest { path: path.clone() }),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::new(),
    )
    .err()
    .expect("a call on a project that is off fails");
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!("project_not_enabled")),
        "{error:?}"
    );
    assert!(
        !activation.exists(),
        "the failed call must not recreate the directory"
    );
    assert_eq!(
        control(&path, ProjectControlAction::Off).0.status,
        "already_disabled",
        "a second off has nothing left to remove"
    );
}

#[test]
fn off_fails_when_the_activation_dir_cannot_be_inspected() {
    let (project, path) = indexed_project();
    // `.planning` is a file, so `.planning/slugaudit` fails with ENOTDIR.
    std::fs::write(project.path().join(".planning"), b"x").expect("write file");

    let error = project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path),
            action: ProjectControlAction::Off,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .expect("an uninspectable activation dir is not 'already disabled'");

    assert_eq!(
        error.message,
        "disable: failed to update the activation directory: Not a directory (os error 20)",
        "an uninspectable activation dir fails the operation with its cause"
    );
}
