//! `structure` is bounded by `max_parse_bytes`, by its byte window before any
//! file is loaded, and by its execution budget during the parse itself.

use super::*;
use crate::tools::PageLimit;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
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
        .map(|index| format!("pub fn f{index}(x: Option<u8>) -> u8 {{ x.expect() }}\n"))
        .collect::<Vec<_>>()
        .concat()
}

#[test]
fn a_multi_file_scan_skips_a_file_over_max_parse_bytes_and_counts_it() -> TestResult {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    std::fs::write(project.path().join("big.rs"), many_functions(10)).check("write big.rs")?;
    let limits = ResourceLimits {
        max_parse_bytes: 100,
        ..ResourceLimits::default()
    };

    let response = call(&multi_file(&project), &limits).check("structure succeeds")?;

    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    ensure_eq(&names, &["alpha"], "names == [\"alpha\"]")?;
    ensure_eq(
        &response.files_over_parse_limit,
        &1,
        "response.files_over_parse_limit == 1",
    )?;
    ensure_eq(
        &response.files_not_scanned,
        &0,
        "response.files_not_scanned == 0",
    )?;
    ensure(
        !response.page.complete,
        "skipped oversize file must mark page incomplete",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::ParseSkipped,
        "response.page.limited_by == PageLimit::ParseSkipped",
    )?;
    ensure_eq(
        &response.page.completeness,
        &Some(crate::tools::Completeness::Partial {
            limited_by: PageLimit::ParseSkipped,
            resume_after_file: None,
            files_scanned: 1,
            files_total: 2,
        }),
        "oversize file skip reports ParseSkipped partial completeness",
    )?;
    Ok(())
}

#[test]
fn a_single_file_over_max_parse_bytes_is_refused_by_name() -> TestResult {
    let project = activated_project("big.rs", many_functions(10).as_bytes())?;
    let limits = ResourceLimits {
        max_parse_bytes: 100,
        ..ResourceLimits::default()
    };

    let error = call(&single_file(&project, "big.rs"), &limits)
        .check_err("an oversize file must be refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure(
        error.message.starts_with("big.rs is ") && error.message.contains("max_parse_bytes (100)"),
        format!("{}", error.message),
    )?;
    Ok(())
}

#[test]
fn the_window_refuses_a_file_larger_than_itself_before_loading_it() -> TestResult {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    std::fs::write(project.path().join("b.rs"), many_functions(2)).check("write b.rs")?;
    std::fs::write(project.path().join("c.rs"), b"pub fn gamma() {}\n").check("write c.rs")?;
    let synced = crate::tools::context::ensure_synced(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("sync")?;
    let connection = crate::store::open_read_write(&synced.database_path).check("open database")?;
    let bounds = super::source::WindowBounds {
        window_bytes: 30,
        max_parse_bytes: u64::MAX,
    };

    let window = super::source::fetch_sources_for_language(&connection, "rust", None, None, bounds)
        .check("the window loads")?;

    let loaded: Vec<&str> = window.files.iter().map(|(path, _)| path.as_str()).collect();
    ensure_eq(
        &loaded,
        &["a.rs"],
        "b.rs is over the window; c.rs no longer fits",
    )?;
    let loaded_bytes: usize = window.files.iter().map(|(_, content)| content.len()).sum();
    ensure(loaded_bytes <= 30, format!("{loaded_bytes} bytes loaded"))?;
    ensure_eq(
        &window.files_over_parse_limit,
        &1,
        "window.files_over_parse_limit == 1",
    )?;
    ensure_eq(
        &window.files_not_scanned,
        &1,
        "window.files_not_scanned == 1",
    )?;
    ensure_eq(
        &window.next_start_after_file.as_deref(),
        &Some("b.rs"),
        "window.next_start_after_file.as_deref() == Some(\"b.rs\")",
    )?;
    Ok(())
}

#[test]
fn a_parse_cancelled_at_the_deadline_returns_earlier_matches_flagged_as_partial() -> TestResult {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    std::fs::write(project.path().join("big.rs"), many_functions(150_000)).check("write big.rs")?;
    let limits = ResourceLimits {
        max_parse_bytes: 64 * 1024 * 1024,
        max_structure_execution_time: Duration::from_millis(100),
        ..ResourceLimits::default()
    };

    let response = call(&multi_file(&project), &limits).check("a partial result, not an error")?;

    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    ensure_eq(&names, &["alpha"], "names == [\"alpha\"]")?;
    ensure_eq(
        &response.parse_timed_out_file.as_deref(),
        &None,
        "earlier files consumed part of the budget, so big.rs must be retried rather than skipped",
    )?;
    ensure_eq(
        &response.next_start_after_file.as_deref(),
        &Some("a.rs"),
        "response.next_start_after_file.as_deref() == Some(\"a.rs\")",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::TimeBudget,
        "response.page.limited_by == PageLimit::TimeBudget",
    )?;
    ensure(!response.page.complete, "the unscanned file remains")?;

    let mut resumed = multi_file(&project);
    resumed.0.start_after_file = response.next_start_after_file;
    let second = call(&resumed, &limits).check("resumed scan marks big.rs timed out")?;
    ensure_eq(
        &second.parse_timed_out_file.as_deref(),
        &Some("big.rs"),
        "big.rs exhausted a full budget on its own",
    )?;
    ensure_eq(
        &second.next_start_after_file.as_deref(),
        &None,
        "no files remain after big.rs so next_start_after_file is None",
    )?;
    ensure_eq(
        &second.page.limited_by,
        &PageLimit::TimeBudget,
        "second.page.limited_by == PageLimit::TimeBudget",
    )?;
    ensure(!second.page.complete, "the timed-out file remains")?;
    Ok(())
}

#[test]
fn a_single_file_parse_cancelled_at_the_deadline_is_the_budget_error() -> TestResult {
    let project = activated_project("big.rs", many_functions(150_000).as_bytes())?;
    let limits = ResourceLimits {
        max_parse_bytes: 64 * 1024 * 1024,
        max_structure_execution_time: Duration::from_millis(100),
        ..ResourceLimits::default()
    };

    let error = call(&single_file(&project, "big.rs"), &limits)
        .check_err("nothing parsed, so nothing to return")?;

    ensure(
        error
            .message
            .starts_with("structure query exceeded its execution time budget")
            && !error.message.contains("SLUGAUDIT_"),
        format!(
            "single-file budget failure states actionable tool remedy without host env vars: {}",
            error.message
        ),
    )?;
    let data = error.data.check("structured budget error payload")?;
    ensure_eq(
        &data.get("code").and_then(serde_json::Value::as_str),
        &Some("structure_budget_exhausted"),
        "carries structure_budget_exhausted code",
    )?;
    Ok(())
}

#[test]
fn a_multi_file_deadline_between_files_returns_a_resumable_partial_page() -> TestResult {
    let project = activated_project("f000.rs", b"pub fn f000() {}\n")?;
    for index in 1..400 {
        let name = format!("f{index:03}.rs");
        let body = format!("pub fn f{index:03}() {{}}\n");
        std::fs::write(project.path().join(name), body).check("write fixture file")?;
    }
    let limits = ResourceLimits {
        max_structure_execution_time: Duration::from_millis(1),
        ..ResourceLimits::default()
    };

    let response = call(&multi_file(&project), &limits)
        .check("expired deadline between files returns partial page, not error")?;

    ensure(
        !response.matches.is_empty(),
        "earlier completed matches preserved",
    )?;
    ensure_eq(
        &response.parse_timed_out_file,
        &None,
        "no single file exhausted the full budget on its own",
    )?;
    ensure(
        response.next_start_after_file.is_some(),
        "partial response names resume_after_file",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::TimeBudget,
        "limited_by reports TimeBudget",
    )?;
    ensure(!response.page.complete, "unscanned files remain")?;
    ensure(
        response.files_not_scanned > 0,
        "mid-window deadline reports remaining files_not_scanned",
    )?;
    ensure(
        response.bytes_not_scanned > 0,
        "mid-window deadline reports remaining bytes_not_scanned",
    )?;
    Ok(())
}

#[test]
fn query_cursor_timeout_leaves_parse_timed_out_file_none() -> TestResult {
    let mut dense = String::new();
    for fn_idx in 0..50 {
        dense.push_str("pub fn dense_");
        dense.push_str(&fn_idx.to_string());
        dense.push_str("() {\n");
        for _ in 0..35 {
            dense.push_str("    let _ = 0;\n");
        }
        dense.push_str("}\n");
    }
    let project = activated_project("dense.rs", dense.as_bytes())?;
    let limits = ResourceLimits {
        max_structure_execution_time: Duration::from_millis(20),
        ..ResourceLimits::default()
    };
    let request = Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: "(block (let_declaration) @a (let_declaration) @b (let_declaration) @c)".to_owned(),
        ..Default::default()
    });

    let response = call(&request, &limits).check("cursor timeout returns partial result")?;

    ensure_eq(
        &response.page.limited_by,
        &PageLimit::TimeBudget,
        "query cursor timeout reports TimeBudget",
    )?;
    ensure_eq(
        &response.parse_timed_out_file,
        &None,
        "query cursor timeout must not be misreported as parse_timed_out_file",
    )?;
    Ok(())
}
