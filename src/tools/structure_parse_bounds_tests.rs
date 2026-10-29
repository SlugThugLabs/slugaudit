//! `structure` is bounded by `max_parse_bytes`, by its byte window before any
//! file is loaded, and by its execution budget during the parse itself (A4).

use super::*;
use crate::tools::PageLimit;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use std::time::Duration;

const QUERY: &str = "(function_item name: (identifier) @name)";

fn multi_file(project: &tempfile::TempDir) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: QUERY.to_owned(),
        full_text: Some(true),
        ..Default::default()
    })
}

fn single_file(project: &tempfile::TempDir, file: &str) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: Some(file.to_owned()),
        query: QUERY.to_owned(),
        ..Default::default()
    })
}

fn call(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
) -> Result<StructureResponse, ErrorData> {
    structure_with_limits(
        request,
        limits,
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

/// `count` one-line Rust functions: a large input whose parse takes far
/// longer than a few milliseconds.
fn many_functions(count: usize) -> String {
    (0..count)
        .map(|index| format!("pub fn f{index}(x: Option<u8>) -> u8 {{ x.unwrap() }}\n"))
        .collect()
}

#[test]
fn a_multi_file_scan_skips_a_file_over_max_parse_bytes_and_counts_it() {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    std::fs::write(project.path().join("big.rs"), many_functions(10)).expect("write big.rs");
    let limits = ResourceLimits {
        max_parse_bytes: 100,
        ..ResourceLimits::default()
    };

    let response = call(&multi_file(&project), &limits).expect("structure succeeds");

    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, ["alpha"]);
    assert_eq!(response.files_over_parse_limit, 1);
    assert_eq!(response.files_not_scanned, 0);
}

#[test]
fn a_single_file_over_max_parse_bytes_is_refused_by_name() {
    let project = activated_project("big.rs", many_functions(10).as_bytes());
    let limits = ResourceLimits {
        max_parse_bytes: 100,
        ..ResourceLimits::default()
    };

    let error = call(&single_file(&project, "big.rs"), &limits)
        .expect_err("an oversize file must be refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.starts_with("big.rs is ") && error.message.contains("max_parse_bytes (100)"),
        "{}",
        error.message
    );
}

#[test]
fn the_window_refuses_a_file_larger_than_itself_before_loading_it() {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    std::fs::write(project.path().join("b.rs"), many_functions(2)).expect("write b.rs");
    std::fs::write(project.path().join("c.rs"), b"pub fn gamma() {}\n").expect("write c.rs");
    let synced = crate::tools::context::ensure_synced(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("sync");
    let connection = crate::store::open_read_write(&synced.database_path).expect("open database");
    let bounds = crate::tools::structure_source::WindowBounds {
        window_bytes: 30,
        max_parse_bytes: u64::MAX,
    };

    let window = crate::tools::structure_source::fetch_sources_for_language(
        &connection,
        "rust",
        None,
        None,
        bounds,
    )
    .expect("the window loads");

    let loaded: Vec<&str> = window.files.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        loaded,
        ["a.rs"],
        "b.rs is over the window; c.rs no longer fits"
    );
    let loaded_bytes: usize = window.files.iter().map(|(_, content)| content.len()).sum();
    assert!(loaded_bytes <= 30, "{loaded_bytes} bytes loaded");
    assert_eq!(window.files_over_parse_limit, 1);
    assert_eq!(window.files_not_scanned, 1);
    assert_eq!(window.next_start_after_file.as_deref(), Some("b.rs"));
}

#[test]
fn a_parse_cancelled_at_the_deadline_returns_earlier_matches_flagged_as_partial() {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    std::fs::write(project.path().join("big.rs"), many_functions(150_000)).expect("write big.rs");
    let limits = ResourceLimits {
        max_parse_bytes: 64 * 1024 * 1024,
        max_structure_execution_time: Duration::from_millis(100),
        ..ResourceLimits::default()
    };

    let response = call(&multi_file(&project), &limits).expect("a partial result, not an error");

    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, ["alpha"]);
    assert_eq!(response.parse_timed_out_file.as_deref(), Some("big.rs"));
    assert_eq!(response.next_start_after_file.as_deref(), Some("big.rs"));
    assert_eq!(response.page.limited_by, PageLimit::TimeBudget);
    assert!(!response.page.complete, "the timed-out file remains");
}

#[test]
fn a_single_file_parse_cancelled_at_the_deadline_is_the_budget_error() {
    let project = activated_project("big.rs", many_functions(150_000).as_bytes());
    let limits = ResourceLimits {
        max_parse_bytes: 64 * 1024 * 1024,
        max_structure_execution_time: Duration::from_millis(100),
        ..ResourceLimits::default()
    };

    let error = call(&single_file(&project, "big.rs"), &limits)
        .expect_err("nothing parsed, so nothing to return");

    assert_eq!(
        error.message.as_ref(),
        "structure query exceeded its execution time budget"
    );
}
