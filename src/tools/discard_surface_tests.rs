//! `database_discard` on tool responses: the call that discarded a project
//! database reports why. Session findings live in their own database, so a
//! discard loses none.

use super::finding::{FindingRequest, finding};
use super::project_control::{ProjectControlAction, ProjectControlRequest, project_control};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult};
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

fn source_project() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

#[test]
fn discard_surface_on_action_reports_a_corrupt_database_and_its_cause() -> TestResult {
    let (project, path) = source_project()?;
    project_control(&on_request(&path), &NoopProgressSink, None).check("first enable succeeds")?;
    fs::write(
        project.path().join(".planning/slugaudit/project.db"),
        b"this is not a sqlite database",
    )
    .check("corrupt the database")?;

    let response =
        project_control(&on_request(&path), &NoopProgressSink, None).check("enable recovers")?;

    let value = serde_json::to_value(&response.0).check("serialize response")?;
    assert!(
        value["database_discard"]
            .get("findings_discarded")
            .is_none(),
        "no findings are lost with the project database: {value}"
    );
    assert!(
        value["database_discard"]["cause"]
            .as_str()
            .check("cause is text")?
            .contains("corrupt or outdated"),
        "{value}"
    );
    Ok(())
}

#[test]
fn discard_surface_on_action_reports_nothing_for_a_healthy_database() -> TestResult {
    let (_project, path) = source_project()?;
    project_control(&on_request(&path), &NoopProgressSink, None).check("first enable succeeds")?;

    let response = project_control(&on_request(&path), &NoopProgressSink, None)
        .check("second enable succeeds")?;

    let value = serde_json::to_value(&response.0).check("serialize response")?;
    assert!(value.get("database_discard").is_none(), "{value}");
    Ok(())
}

#[test]
fn discard_surface_finding_response_carries_the_rebuild_cause_and_keeps_the_finding() -> TestResult
{
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    let first = finding(&finding_request(&project), &NoopProgressSink, &manager)
        .check("first finding succeeds")?;
    assert!(
        serde_json::to_value(&first.0)
            .check("serialize")?
            .get("database_discard")
            .is_none(),
        "a normal call reports no discard"
    );
    let database = project.path().join(".planning/slugaudit/project.db");
    Connection::open(&database)
        .check("raw open")?
        .execute_batch("PRAGMA user_version = 999;")
        .check("foreign schema version")?;

    let second = finding(&finding_request(&project), &NoopProgressSink, &manager)
        .check("finding recovers")?;

    let value = serde_json::to_value(&second.0).check("serialize")?;
    assert!(
        value["database_discard"]["cause"]
            .as_str()
            .check("cause is text")?
            .contains("corrupt or outdated"),
        "{value}"
    );
    assert_eq!(
        second.0.id, first.0.id,
        "the restated finding refreshes the one the rebuild left in the findings file"
    );
    Ok(())
}
