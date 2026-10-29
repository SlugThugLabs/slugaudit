//! `report` keeps prose out of the parser-failure signal.

use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

#[test]
fn prose_files_are_counted_but_never_listed_as_parser_failures() {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).expect("activate");
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"));
    fs::write(project.path().join("a.md"), b"# title\n").expect("write md");
    fs::write(project.path().join("b.txt"), b"notes\n").expect("write txt");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write rs");
    fs::write(project.path().join("c.zzunknown"), b"data\n").expect("write unknown");

    let response = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("report succeeds")
    .0;

    let listed: Vec<&str> = response
        .parser_failure_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(listed, ["c.zzunknown"]);
    assert_eq!(response.parser_failure_count, 1);
    assert_eq!(response.parser_failure_files_total, 1);
    assert_eq!(response.completeness.prose_files_not_parsed, 2);
}

#[test]
fn a_freshly_enabled_project_reports_no_parser_failure_for_its_generated_gitignore() {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join("src")).expect("src dir");
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").expect("write rs");
    let path = project.path().to_string_lossy().into_owned();
    let manager = crate::sync::SourceSyncManager::default();

    crate::tools::project_control(
        &Parameters(crate::tools::ProjectControlRequest {
            path: Some(path.clone()),
            action: crate::tools::ProjectControlAction::On,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("project_control on succeeds");
    assert!(
        project.path().join(".gitignore").is_file(),
        "`on` wrote the .gitignore"
    );
    let response = report(
        &Parameters(ReportRequest { path }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("report succeeds")
    .0;

    assert_eq!(response.parser_failure_count, 0);
    assert!(response.parser_failure_files.is_empty());
    assert_eq!(response.completeness.prose_files_not_parsed, 1);
}
