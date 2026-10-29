//! Tests for what `project_control` `off` reports about the findings it
//! discards, and for the tool's destructive annotation.

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
        profile: None,
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

fn seed_findings(project: &tempfile::TempDir, count: usize) {
    let connection =
        store::open_read_write(&activation_dir(project).join("project.db")).expect("open db");
    for index in 0..count {
        connection
            .execute(
                "INSERT INTO findings (path, source_hash, line_start, line_end, \
                 severity, category, title, description, created_at_unix, \
                 evidence_revision, status, session_id) \
                 VALUES (?1, 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', '')",
                rusqlite::params![format!("f{index}.rs")],
            )
            .expect("seed finding");
    }
}

#[test]
fn off_action_reports_the_findings_it_discards_and_keeps_config() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None).expect("enable");
    seed_findings(&project, 3);
    fs::write(activation_dir(&project).join("config.json"), b"{}").expect("write config");

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("disable succeeds");

    assert_eq!(response.0.status, "disabled");
    assert_eq!(response.0.findings_discarded, Some(3));
    assert!(response.0.config_kept);
    assert!(!activation_dir(&project).join("project.db").exists());
    assert!(activation_dir(&project).join("config.json").is_file());
}

#[test]
fn off_action_with_no_findings_reports_zero() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None).expect("enable");

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("disable succeeds");

    assert_eq!(response.0.findings_discarded, Some(0));
    assert!(!activation_dir(&project).exists());
}

#[test]
fn project_control_tool_is_annotated_destructive() {
    let tool = crate::server::tool_surface()
        .into_iter()
        .find(|tool| tool.name == "project_control")
        .expect("project_control is advertised");
    let annotations = tool.annotations.expect("project_control has annotations");
    assert_eq!(annotations.destructive_hint, Some(true));
    assert_eq!(annotations.read_only_hint, Some(false));
}
