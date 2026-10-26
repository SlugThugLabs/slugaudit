//! A file whose grammar did not run is visible in every `leads` response
//! (`files_not_parsed`) and in `report.completeness`, so a mechanic's count
//! is never quoted as complete while a file was left unparsed (A4).

use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;
use std::time::{Duration, Instant};

/// A 3 MiB Rust file (over the 2 MiB `max_parse_bytes` default) full of
/// `unwrap()` calls, beside one small parsed file with exactly one.
fn oversize_fixture() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).expect("activate");
    let line = "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n";
    let big = line.repeat(3 * 1024 * 1024 / line.len() + 1);
    assert!(big.len() > 3 * 1024 * 1024, "the fixture is over 3 MiB");
    fs::write(project.path().join("big.rs"), big).expect("write oversize file");
    fs::write(project.path().join("small.rs"), line).expect("write small file");
    project
}

fn leads_request(project: &tempfile::TempDir, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: project.path().to_string_lossy().into_owned(),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        offset: None,
        expected_revision: None,
        mode: LeadMode::Full,
        page_bytes: None,
    }
}

fn run_leads(request: LeadsRequest, manager: &crate::sync::SourceSyncManager) -> LeadsResponse {
    leads(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .expect("leads succeeds")
    .0
}

#[test]
fn an_oversize_file_indexes_fast_and_every_leads_response_counts_it_not_parsed() {
    let project = oversize_fixture();
    let manager = crate::sync::SourceSyncManager::default();

    let started = Instant::now();
    let counts = run_leads(leads_request(&project, None), &manager);
    let elapsed = started.elapsed();
    let rows = run_leads(leads_request(&project, Some("unwrap_calls")), &manager);
    let compact = run_leads(
        LeadsRequest {
            mode: LeadMode::Compact,
            ..leads_request(&project, None)
        },
        &manager,
    );

    assert!(
        elapsed < Duration::from_secs(2),
        "indexing took {elapsed:?}"
    );
    assert_eq!(counts.files_not_parsed, 1, "counts mode");
    assert_eq!(rows.files_not_parsed, 1, "rows mode");
    assert_eq!(compact.files_not_parsed, 1, "compact mode");
    assert_eq!(rows.total, 1, "only small.rs was parsed");
    let LeadRows::Full(rows) = &rows.rows else {
        panic!("full mode returns full rows");
    };
    assert_eq!(rows[0].file, "small.rs");
}

#[test]
fn files_not_parsed_follows_the_request_pattern() {
    let project = oversize_fixture();
    let manager = crate::sync::SourceSyncManager::default();

    let narrowed = run_leads(
        LeadsRequest {
            pattern: Some("small".to_owned()),
            ..leads_request(&project, None)
        },
        &manager,
    );

    assert_eq!(narrowed.files_not_parsed, 0);
}

#[test]
fn report_completeness_counts_an_oversize_for_parse_file_and_lists_its_reason() {
    let project = oversize_fixture();

    let response = crate::tools::report(
        &Parameters(crate::tools::ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("report succeeds")
    .0;

    assert_eq!(response.completeness.oversize_for_parse_file_count, 1);
    assert_eq!(response.completeness.parse_timeout_file_count, 0);
    let listed: Vec<(&str, Option<&str>)> = response
        .parser_failure_files
        .iter()
        .map(|file| (file.path.as_str(), file.parse_error_reason.as_deref()))
        .collect();
    assert_eq!(listed, [("big.rs", Some("oversize_for_parse"))]);
}

#[test]
fn a_file_with_no_detected_language_is_not_counted_as_not_parsed() {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).expect("activate");
    fs::write(project.path().join(".gitignore"), "target/\n").expect("write .gitignore");
    fs::write(project.path().join("small.rs"), "pub fn f() {}\n").expect("write small file");
    let manager = crate::sync::SourceSyncManager::default();

    let response = run_leads(leads_request(&project, None), &manager);

    assert_eq!(response.files_not_parsed, 0);
}
