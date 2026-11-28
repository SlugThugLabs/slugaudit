//! `report` keeps prose out of the parser-failure signal.

use super::*;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

#[test]
fn prose_files_are_counted_but_never_listed_as_parser_failures() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).check("activate")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    fs::write(project.path().join("a.md"), b"# title\n").check("write md")?;
    fs::write(project.path().join("b.txt"), b"notes\n").check("write txt")?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write rs")?;
    fs::write(project.path().join("c.zzunknown"), b"data\n").check("write unknown")?;

    let response = report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("report succeeds")?
    .0;

    let listed: Vec<&str> = response
        .parser_failure_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    ensure_eq(&listed, &["c.zzunknown"], "listed == [\"c.zzunknown\"]")?;
    ensure_eq(
        &response.parser_failure_count,
        &1,
        "response.parser_failure_count == 1",
    )?;
    ensure_eq(
        &response.completeness.prose_files_not_parsed,
        &2,
        "response.completeness.prose_files_not_parsed == 2",
    )?;
    Ok(())
}

#[test]
fn a_freshly_enabled_project_reports_no_parser_failure_for_its_generated_gitignore() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join("src")).check("src dir")?;
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").check("write rs")?;
    let path = project.path().to_string_lossy().into_owned();
    let manager = crate::sync::SourceSyncManager::default();

    crate::tools::project_control(
        &Parameters(crate::tools::ProjectControlRequest {
            path: Some(path.clone()),
            action: crate::tools::ProjectControlAction::On,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("project_control on succeeds")?;
    ensure(
        project.path().join(".gitignore").is_file(),
        "`on` wrote the .gitignore",
    )?;
    let response = report(
        &Parameters(ReportRequest { path }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("report succeeds")?
    .0;

    ensure_eq(
        &response.parser_failure_count,
        &0,
        "response.parser_failure_count == 0",
    )?;
    ensure(
        response.parser_failure_files.is_empty(),
        "response.parser_failure_files.is_empty()",
    )?;
    ensure_eq(
        &response.completeness.prose_files_not_parsed,
        &1,
        "the generated .gitignore is the only prose file not parsed (.planning/ is excluded)",
    )?;
    Ok(())
}

fn report_of(files: &[(&str, &[u8])]) -> TestResult<ReportResponse> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).check("activate")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    for (relative, content) in files {
        fs::write(project.path().join(relative), content).check("write fixture file")?;
    }
    Ok(report(
        &Parameters(ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("report succeeds")?
    .0)
}

#[test]
fn an_untested_language_is_flagged_and_gets_exactly_one_notice() -> TestResult {
    let response = report_of(&[
        ("lib.rs", b"pub fn a() {}\n"),
        ("tool.py", b"def b():\n    return 1\n"),
        ("NOTES.md", b"# notes\n"),
    ])?;

    let tested: Vec<(&str, bool)> = response
        .languages
        .iter()
        .map(|count| (count.language.as_str(), count.tested))
        .collect();
    ensure_eq(
        &tested,
        &[("prose", false), ("python", false), ("rust", true)],
        "tested == [(\"prose\", false), (\"python\", false), (\"rust\", true)]",
    )?;
    ensure_eq(
        &response.notices,
        &["python: 1 file uses an untested grammar. \
             Only Rust is tested; treat python results as best-effort."],
        "response.notices == [ \"python: 1 file uses an untested grammar. \\ Only Rust is tested; treat python results as best-effort.\" ]",
    )?;
    Ok(())
}

#[test]
fn a_rust_only_project_has_no_notices() -> TestResult {
    let response = report_of(&[("lib.rs", b"pub fn a() {}\n"), ("NOTES.md", b"# notes\n")])?;

    ensure(
        response.notices.is_empty(),
        format!("{:?}", response.notices),
    )?;
    Ok(())
}
