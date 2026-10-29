//! `database_discard` on tool responses: the call that discarded a project
//! database reports how many findings were lost (null when unreadable).

use super::finding::{FindingRequest, finding};
use super::project_control::{ProjectControlAction, ProjectControlRequest, project_control};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use std::fs;

fn on_request(path: &str) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action: ProjectControlAction::On,
        profile: None,
    })
}

fn finding_request(project: &tempfile::TempDir) -> Parameters<FindingRequest> {
    Parameters(FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: "lib.rs".to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "medium".to_owned(),
        category: "correctness".to_owned(),
        title: "Reviewed conclusion".to_owned(),
        description: "A conclusion the AI actually checked.".to_owned(),
    })
}

fn source_project() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

#[test]
fn discard_surface_on_action_reports_a_corrupt_database_as_unknown() {
    let (project, path) = source_project();
    project_control(&on_request(&path), &NoopProgressSink, None).expect("first enable succeeds");
    fs::write(
        project.path().join(".planning/slugaudit/project.db"),
        b"this is not a sqlite database",
    )
    .expect("corrupt the database");

    let response =
        project_control(&on_request(&path), &NoopProgressSink, None).expect("enable recovers");

    let value = serde_json::to_value(&response.0).expect("serialize response");
    assert!(
        value["database_discard"]["findings_discarded"].is_null(),
        "unreadable database reports null, not 0: {value}"
    );
    assert!(
        value["database_discard"]["cause"]
            .as_str()
            .expect("cause is text")
            .contains("corrupt or outdated"),
        "{value}"
    );
}

#[test]
fn discard_surface_on_action_reports_nothing_for_a_healthy_database() {
    let (_project, path) = source_project();
    project_control(&on_request(&path), &NoopProgressSink, None).expect("first enable succeeds");

    let response = project_control(&on_request(&path), &NoopProgressSink, None)
        .expect("second enable succeeds");

    let value = serde_json::to_value(&response.0).expect("serialize response");
    assert!(value.get("database_discard").is_none(), "{value}");
}

#[test]
fn discard_surface_finding_response_carries_the_rebuild_count() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let manager = SourceSyncManager::default();
    let first = finding(&finding_request(&project), &NoopProgressSink, &manager)
        .expect("first finding succeeds");
    assert!(
        serde_json::to_value(&first.0)
            .expect("serialize")
            .get("database_discard")
            .is_none(),
        "a normal call reports no discard"
    );
    let database = project.path().join(".planning/slugaudit/project.db");
    Connection::open(&database)
        .expect("raw open")
        .execute_batch("PRAGMA user_version = 999;")
        .expect("foreign schema version");

    let second =
        finding(&finding_request(&project), &NoopProgressSink, &manager).expect("finding recovers");

    let value = serde_json::to_value(&second.0).expect("serialize");
    assert_eq!(
        value["database_discard"]["findings_discarded"], 1,
        "{value}"
    );
}
