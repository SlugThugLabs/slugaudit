//! Tests for the `structure` tool's limit and edge branches: the empty
//! query rejection, a file with no indexed content (binary), and the
//! long-match-text truncation path. Kept separate from `structure_tests.rs`
//! so each file stays under the source-size cap.

use super::*;
use crate::tools::PageLimit;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn ask(
    project: &tempfile::TempDir,
    file: &str,
    query: &str,
) -> Result<StructureResponse, ErrorData> {
    structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: Some(file.to_owned()),
            language: None,
            pattern: None,
            query: query.to_owned(),
            full_text: None,
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn an_empty_query_is_a_typed_error_not_a_silent_noop() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let result = ask(&project, "lib.rs", "   \n  ");
    let error = result.check_err("a whitespace-only query must be rejected")?;
    ensure(
        error.message.contains("must not be empty"),
        "error.message.contains(\"must not be empty\")",
    )?;
    Ok(())
}

#[test]
fn a_file_without_indexed_content_is_a_typed_error() -> TestResult {
    let project = activated_project("logo.bin", b"\x89PNG\x00\x01\x02\x00\x00")?;
    let result = ask(&project, "logo.bin", "(identifier) @x");
    let error =
        result.check_err("a binary file with no indexed source content must not be queried")?;
    ensure(
        error.message.contains("no indexed source content") || error.message.contains("logo.bin"),
        format!("unexpected error message: {}", error.message),
    )?;
    Ok(())
}

/// Match text longer than `MAX_TEXT_BYTES` must be truncated at a UTF-8
/// boundary and flagged with `text_truncated`, never emitted in full and
/// never panicking on the boundary slice.
#[test]
fn long_match_text_is_truncated_and_flagged() -> TestResult {
    let mut content = String::with_capacity(1_100_000);
    content.push_str("let long = \"");
    content.push_str(&"abcdefghij".repeat(105_000));
    content.push_str("\";\n");
    let project = activated_project("lib.rs", content.as_bytes())?;

    let response = ask(&project, "lib.rs", "(string_literal) @s").check("query succeeds")?;
    let first = &response.matches.first().check("response.matches[0]")?;
    ensure(
        first.text_truncated,
        "a >1MB match must be flagged as truncated",
    )?;
    ensure(
        first.text.len() <= super::MAX_TEXT_BYTES,
        "truncated text must respect the byte cap",
    )?;
    ensure(
        first.text.is_char_boundary(first.text.len()),
        "truncation must land on a UTF-8 boundary",
    )?;
    Ok(())
}

pub(super) fn many_functions(count: usize) -> String {
    (0..count)
        .map(|index| format!("pub fn f{index:02}() {{}}\n"))
        .collect::<Vec<_>>()
        .concat()
}

pub(super) fn page_request(
    project: &tempfile::TempDir,
    cursor: Option<&str>,
    page_bytes: Option<usize>,
) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: Some("lib.rs".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        cursor: cursor.map(str::to_owned),
        page_bytes,
        ..Default::default()
    })
}

pub(super) fn run_with(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    window_bytes: usize,
) -> TestResult<StructureResponse> {
    run_with_manager(
        request,
        limits,
        window_bytes,
        &crate::sync::SourceSyncManager::default(),
    )
}

/// Like [`run_with`], on a manager the caller keeps so a cursor resolves.
pub(super) fn run_with_manager(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    window_bytes: usize,
    manager: &crate::sync::SourceSyncManager,
) -> TestResult<StructureResponse> {
    structure_with_limits(
        request,
        limits,
        window_bytes,
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
    .check("structure call succeeds")
}

#[test]
fn a_zero_page_bytes_is_refused_rather_than_substituted() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;

    let result = structure(
        &page_request(&project, None, Some(0)),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    ensure(result.is_err(), "result.is_err()")?;
    Ok(())
}

#[test]
fn start_after_file_with_a_single_file_query_is_refused() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = page_request(&project, None, None);
    request.0.start_after_file = Some("a.rs".to_owned());

    let result = structure(
        &request,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    let error = result.err().check("start_after_file needs multi-file")?;
    ensure(
        error.message.contains("start_after_file"),
        format!("{}", error.message),
    )?;
    Ok(())
}

#[test]
fn files_past_the_memory_window_are_reported_and_a_continuation_reaches_them() -> TestResult {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    for (name, body) in [
        ("b.rs", "pub fn beta() {}\n"),
        ("c.rs", "pub fn gamma() {}\n"),
    ] {
        std::fs::write(project.path().join(name), body).check("write fixture")?;
    }
    let limits = ResourceLimits::default();
    let mut names = Vec::new();
    let mut start_after: Option<String> = None;
    let mut windows = 0;

    loop {
        let request = Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            language: Some("rust".to_owned()),
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: Some(true),
            start_after_file: start_after.clone(),
            ..Default::default()
        });
        // Each fixture file is 17 or 18 bytes, so an 18-byte window holds
        // exactly one of them.
        let response = run_with(&request, &limits, 18)?;
        windows += 1;
        names.extend(response.matches.iter().map(|m| m.text.clone()));
        if let Some(next) = response.next_start_after_file {
            ensure(
                response.files_not_scanned > 0 && response.bytes_not_scanned > 0,
                "response.files_not_scanned > 0 && response.bytes_not_scanned > 0",
            )?;
            ensure(!response.page.complete, "a window remains")?;
            ensure_eq(
                &response.page.limited_by,
                &PageLimit::ScanWindow,
                "response.page.limited_by == PageLimit::ScanWindow",
            )?;
            ensure_eq(&response.page.cursor, &None, "response.page.cursor == None")?;
            start_after = Some(next);
        } else {
            ensure_eq(
                &response.files_not_scanned,
                &0,
                "response.files_not_scanned == 0",
            )?;
            ensure_eq(
                &response.bytes_not_scanned,
                &0,
                "response.bytes_not_scanned == 0",
            )?;
            ensure(response.page.complete, "response.page.complete")?;
            break;
        }
    }

    ensure_eq(
        &names,
        &["alpha", "beta", "gamma"],
        "names == [\"alpha\", \"beta\", \"gamma\"]",
    )?;
    ensure_eq(&windows, &3, "windows == 3")?;
    Ok(())
}

#[test]
fn a_window_that_ends_exactly_at_the_last_file_reports_nothing_unscanned() -> TestResult {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    let request = Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        ..Default::default()
    });

    // "pub fn alpha() {}\n" is 18 bytes: the window ends exactly at it.
    let response = run_with(&request, &ResourceLimits::default(), 18)?;

    ensure_eq(
        &response.files_not_scanned,
        &0,
        "response.files_not_scanned == 0",
    )?;
    ensure_eq(
        &response.next_start_after_file,
        &None,
        "response.next_start_after_file == None",
    )?;
    ensure_eq(&response.matches.len(), &1, "response.matches.len() == 1")?;
    Ok(())
}

fn multi_file_request(
    project: &tempfile::TempDir,
    cursor: Option<&str>,
) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        cursor: cursor.map(str::to_owned),
        ..Default::default()
    })
}

/// A page cut inside a multi-file scan window leaves `files_not_scanned` at
/// 0 even though later loaded files are not on this page. The cut is
/// signaled by `page.cursor` and `page.limited_by` — `files_not_scanned` only
/// counts files the memory window left unloaded — and the next page continues
/// into the later file.
#[test]
fn a_page_cut_inside_a_multi_file_window_is_signaled_by_the_cursor_not_files_not_scanned()
-> TestResult {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    std::fs::write(project.path().join("b.rs"), b"pub fn beta() {}\n").check("write b.rs")?;
    let limits = ResourceLimits {
        max_structure_matches: 1,
        ..ResourceLimits::default()
    };
    let manager = crate::sync::SourceSyncManager::default();

    let first = run_with_manager(
        &multi_file_request(&project, None),
        &limits,
        usize::MAX,
        &manager,
    )?;

    ensure_eq(
        &first.page.limited_by,
        &PageLimit::MatchCap,
        "the page stopped at the match cap inside a.rs",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;
    ensure_eq(&first.files_not_scanned, &0, "first.files_not_scanned == 0")?;
    ensure_eq(
        &first.next_start_after_file,
        &None,
        "first.next_start_after_file == None",
    )?;
    ensure_eq(&first.matches.len(), &1, "first.matches.len() == 1")?;
    ensure_eq(
        &first.matches.first().check("first.matches[0]")?.file,
        &"a.rs",
        "first.matches[0].file == \"a.rs\"",
    )?;

    let second = run_with_manager(
        &multi_file_request(&project, first.page.cursor.as_deref()),
        &limits,
        usize::MAX,
        &manager,
    )?;

    ensure(second.page.complete, "second.page.complete")?;
    ensure_eq(&second.matches.len(), &1, "second.matches.len() == 1")?;
    ensure_eq(
        &second.matches.first().check("second.matches[0]")?.file,
        &"b.rs",
        "second.matches[0].file == \"b.rs\"",
    )?;
    ensure_eq(
        &second.matches.first().check("second.matches[0]")?.text,
        &"beta",
        "second.matches[0].text == \"beta\"",
    )?;
    Ok(())
}

fn single_file_request(project: &tempfile::TempDir) -> StructureRequest {
    StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: Some("lib.rs".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        ..Default::default()
    }
}

fn refusal(request: StructureRequest) -> TestResult<ErrorData> {
    structure(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("the request must be refused")
}

#[test]
fn a_path_over_the_cap_is_refused_before_any_sync() -> TestResult {
    let mut request = single_file_request(&activated_project("lib.rs", b"pub fn a() {}\n")?);
    request.path = "x".repeat(32 * 1024 + 1);

    let error = structure_with_limits(
        &Parameters(request),
        &ResourceLimits::default(),
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("an oversized path must be refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"path exceeds 32768 bytes",
        "error.message.as_ref() == \"path exceeds 32768 bytes\"",
    )?;
    Ok(())
}

#[test]
fn an_oversized_start_after_file_is_refused_with_its_cap() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = single_file_request(&project);
    request.file = None;
    request.language = Some("rust".to_owned());
    request.start_after_file = Some("x".repeat(32 * 1024 + 1));

    let error = refusal(request)?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"start_after_file exceeds 32768 bytes",
        "error.message.as_ref() == \"start_after_file exceeds 32768 bytes\"",
    )?;
    Ok(())
}

#[test]
fn a_cursor_over_the_shared_cursor_cap_is_refused_with_that_cap() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = single_file_request(&project);
    request.cursor = Some("c".repeat(crate::tools::MAX_CURSOR_BYTES + 1));

    let error = refusal(request)?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"cursor exceeds 1024 bytes",
        "error.message.as_ref() == \"cursor exceeds 1024 bytes\"",
    )?;
    Ok(())
}

#[test]
fn an_oversized_file_filter_is_refused_by_name() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = single_file_request(&project);
    request.file = Some("x".repeat(8 * 1024 + 1));

    let error = refusal(request)?;

    ensure_eq(
        &error.message.as_ref(),
        &"file exceeds 8192 bytes",
        "error.message.as_ref() == \"file exceeds 8192 bytes\"",
    )?;
    Ok(())
}

#[test]
fn an_oversized_language_filter_is_refused_by_name() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = single_file_request(&project);
    request.file = None;
    request.language = Some("x".repeat(8 * 1024 + 1));

    let error = refusal(request)?;

    ensure_eq(
        &error.message.as_ref(),
        &"language exceeds 8192 bytes",
        "error.message.as_ref() == \"language exceeds 8192 bytes\"",
    )?;
    Ok(())
}

#[test]
fn an_oversized_pattern_filter_is_refused_by_name() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = single_file_request(&project);
    request.pattern = Some("x".repeat(8 * 1024 + 1));

    let error = refusal(request)?;

    ensure_eq(
        &error.message.as_ref(),
        &"pattern exceeds 8192 bytes",
        "error.message.as_ref() == \"pattern exceeds 8192 bytes\"",
    )?;
    Ok(())
}

#[test]
fn an_unrepresentable_execution_budget_means_no_deadline_not_a_panic() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let limits = ResourceLimits {
        max_structure_execution_time: std::time::Duration::MAX,
        ..ResourceLimits::default()
    };

    let response = run_with(&page_request(&project, None, None), &limits, usize::MAX)?;

    ensure_eq(&response.matches.len(), &1, "response.matches.len() == 1")?;
    Ok(())
}

#[path = "structure_limit_query_tests.rs"]
mod query_limit_tests;
