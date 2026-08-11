//! Tests for the `project_control` MCP tool: `on` creates the activation
//! marker and runs the initial import, `off` purges the marker and the
//! database, and both degrade gracefully when the project is already in
//! the requested state.

use super::*;
use std::fs;

fn temp_project_with_source() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join("src")).expect("create src dir");
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

fn request(path: &str, action: ProjectControlAction) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action,
    })
}

fn on_request(path: &str) -> Parameters<ProjectControlRequest> {
    request(path, ProjectControlAction::On)
}

fn off_request(path: &str) -> Parameters<ProjectControlRequest> {
    request(path, ProjectControlAction::Off)
}

fn activation_dir(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning").join("slugaudit")
}

#[test]
fn on_action_parses() {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(req.action, ProjectControlAction::On));
}

#[test]
fn off_action_parses() {
    let json = r#"{"action":"off"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(req.action, ProjectControlAction::Off));
}

#[test]
fn path_defaults_to_none() {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).unwrap();
    assert!(req.path.is_none());
}

#[test]
fn on_action_enables_the_project_and_runs_the_initial_import() {
    let (project, path) = temp_project_with_source();
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink)
        .expect("enable succeeds");

    let inner = response.0;
    assert_eq!(inner.status, "enabled");
    assert_eq!(
        inner.path,
        project.path().canonicalize().unwrap().to_string_lossy(),
        "the response reports the canonical project root"
    );
    let import = inner.import.expect("import report present");
    assert!(
        import.added >= 1,
        "the initial import must index the source file"
    );
    assert!(!import.revision_id.is_empty());

    assert!(
        activation_dir(&project).is_dir(),
        "the activation marker must exist after enable"
    );
    assert!(
        activation_dir(&project).join("project.db").exists(),
        "the project database must exist after enable"
    );
}

#[test]
fn on_action_is_idempotent_when_already_enabled() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink).expect("first enable");
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink)
        .expect("second enable");
    assert_eq!(response.0.status, "enabled");
    assert!(
        activation_dir(&project).is_dir(),
        "the marker stays in place across repeated enables"
    );
}

#[test]
fn off_action_disables_the_project_and_purges_the_database() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink).expect("enable");
    assert!(activation_dir(&project).is_dir());

    let response = project_control(&off_request(&path), &crate::progress::NoopProgressSink)
        .expect("disable succeeds");
    assert_eq!(response.0.status, "disabled");
    assert!(response.0.import.is_none());
    assert!(
        !activation_dir(&project).exists(),
        "disable must purge the activation marker and the database together"
    );
}

#[test]
fn off_action_when_already_disabled_is_a_noop() {
    let (project, path) = temp_project_with_source();
    let response = project_control(&off_request(&path), &crate::progress::NoopProgressSink)
        .expect("noop disable");
    assert_eq!(response.0.status, "already_disabled");
    assert!(response.0.import.is_none());
    assert!(!activation_dir(&project).exists());
}

#[test]
fn on_action_with_a_nonexistent_path_returns_an_error() {
    let missing = "/definitely/not/a/real/path/for/slugaudit-tests";
    let result = project_control(&on_request(missing), &crate::progress::NoopProgressSink);
    assert!(
        result.is_err(),
        "enable on a nonexistent path must surface an error, not silently create state"
    );
}
