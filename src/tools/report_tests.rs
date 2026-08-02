use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

fn activate(root: &std::path::Path) {
    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("create activation dir");
}

#[test]
fn reports_real_counts_for_an_active_project() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() {}\npub fn b() {}\n",
    )
    .expect("write fixture");

    let response = report(&Parameters(ReportRequest {
        path: project.path().to_string_lossy().into_owned(),
    }))
    .expect("report succeeds");

    assert_eq!(response.0.file_count, 1);
    assert!(
        response
            .0
            .languages
            .iter()
            .any(|entry| entry.language == "rust")
    );
    assert!(
        response
            .0
            .evidence_counts
            .iter()
            .any(|entry| entry.kind == "Structure")
    );
    assert_eq!(response.0.parser_failure_count, 0);
    assert_eq!(response.0.open_finding_count, 0);
}

#[test]
fn an_inactive_project_is_a_typed_error_not_a_panic() {
    let project = tempfile::tempdir().expect("project dir");
    let result = report(&Parameters(ReportRequest {
        path: project.path().to_string_lossy().into_owned(),
    }));
    assert!(result.is_err());
}
