//! Capture-name, UTF-8 and query-size limits of the structure tool.
use super::limit_tests::ask;
use super::*;
use crate::tools::test_support::activated_project;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn rust_root(source: &str) -> TestResult<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language_pack::get_language("rust").check("rust grammar")?)
        .check("set language")?;
    parser.parse(source, None).check("parse")
}

#[test]
fn a_capture_without_a_name_is_logged_and_flagged_as_failed_extraction() -> TestResult {
    let source = "fn a() {}";
    let tree = rust_root(source)?;
    let lines = crate::model::LineIndex::new(source).check("index source")?;

    let (found, logs) = crate::store::test_capture::capture_warns(|| {
        super::match_node::node_to_match("lib.rs", tree.root_node(), source, &lines, None, false)
    })?;
    let found = found.check("map the capture")?;

    ensure(found.extraction_failed, "found.extraction_failed")?;
    ensure(
        logs.contains("capture has no name"),
        format!("logs: {logs}"),
    )?;
    Ok(())
}

#[test]
fn text_that_is_not_valid_utf8_keeps_its_cause_in_the_log() -> TestResult {
    let tree = rust_root("abc")?;
    // Three bytes of a four-byte string end inside the second character.
    let content = "\u{e9}\u{e9}";
    let lines = crate::model::LineIndex::new(content).check("index content")?;

    let (found, logs) = crate::store::test_capture::capture_warns(|| {
        super::match_node::node_to_match(
            "lib.rs",
            tree.root_node(),
            content,
            &lines,
            Some("name"),
            false,
        )
    })?;
    let found = found.check("map the capture")?;

    ensure(found.extraction_failed, "found.extraction_failed")?;
    ensure(found.text.is_empty(), "found.text.is_empty()")?;
    ensure(logs.contains("not valid UTF-8"), format!("logs: {logs}"))?;
    ensure(logs.contains("utf-8"), format!("source error kept: {logs}"))?;
    Ok(())
}

fn query_limit(bytes: usize) -> ResourceLimits {
    ResourceLimits {
        max_structure_query_bytes: bytes,
        ..ResourceLimits::default()
    }
}

#[test]
fn a_query_one_byte_over_the_limit_is_refused_by_size_before_anything_else_runs() -> TestResult {
    let request = Parameters(StructureRequest {
        // Not a project: a size refusal happens before the sync, so the path
        // is never opened.
        path: "/no/such/project".to_owned(),
        file: Some("lib.rs".to_owned()),
        query: "x".repeat(33),
        ..Default::default()
    });

    let error = structure_with_limits(
        &request,
        &query_limit(32),
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
    .check_err("33 bytes is over a 32-byte limit")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &"structure query is 33 bytes; the limit is 32 (max_structure_query_bytes)",
        "error.message == \"structure query is 33 bytes; the limit is 32 (max_structure_query_bytes)\"",
    )?;
    Ok(())
}

#[test]
fn a_query_of_exactly_the_limit_is_not_refused_for_size() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let query = format!("(identifier) @name{}", " ".repeat(14));
    ensure_eq(&query.len(), &32, "query.len() == 32")?;

    let response = structure_with_limits(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: Some("lib.rs".to_owned()),
            query,
            ..Default::default()
        }),
        &query_limit(32),
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
    .check("a query at the limit compiles and runs")?;

    ensure_eq(&response.matches.len(), &1, "response.matches.len() == 1")?;
    Ok(())
}

#[test]
fn a_global_config_query_limit_is_honoured_by_the_tool() -> TestResult {
    let _lock = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let _config =
        crate::tools::test_support::use_global_config(r#"{"max_structure_query_bytes": 20}"#)?;

    let error = ask(
        &project,
        "lib.rs",
        "(function_item name: (identifier) @name)",
    )
    .check_err("the 40-byte query is over the configured 20")?;

    ensure_eq(
        &error.message,
        &"structure query is 40 bytes; the limit is 20 (max_structure_query_bytes)",
        "error.message == \"structure query is 40 bytes; the limit is 20 (max_structure_query_bytes)\"",
    )?;
    Ok(())
}

fn collect_capped_pages(
    limits: &ResourceLimits,
    build_req: impl Fn(Option<&str>) -> Parameters<StructureRequest>,
    format_match: impl Fn(&StructureMatch) -> String,
) -> TestResult<(usize, Vec<String>)> {
    use super::limit_tests::run_with_manager;
    use crate::tools::PageLimit;

    let manager = crate::sync::SourceSyncManager::default();
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0usize;
    loop {
        let page = run_with_manager(&build_req(cursor.as_deref()), limits, usize::MAX, &manager)?;
        pages = pages.checked_add(1).check("page count")?;
        ensure(
            page.matches.len() <= limits.max_structure_matches,
            "each page obeys max_structure_matches",
        )?;
        ensure_eq(
            &page.page.completeness,
            &Some(crate::tools::Completeness::Complete),
            "per-page match cap preserves Complete scan completeness",
        )?;
        ensure_eq(
            &page.next_start_after_file,
            &None,
            "cursor handles continuation, not start_after_file",
        )?;
        seen.extend(page.matches.iter().map(&format_match));
        if let Some(next) = page.page.cursor {
            ensure_eq(
                &page.page.limited_by,
                &PageLimit::MatchCap,
                "MatchCap reported",
            )?;
            ensure(!page.page.complete, "incomplete while cursor remains")?;
            cursor = Some(next);
        } else {
            ensure(page.page.complete, "final page is complete")?;
            return Ok((pages, seen));
        }
    }
}

/// `max_structure_matches` caps each page's row count while streaming all scan
/// matches through `paginate_rows`, allowing multi-page cursor continuation on
/// both single-file and multi-file queries with zero duplicates and zero
/// file-0 stranding.
#[test]
fn max_structure_matches_pages_across_cursor_on_single_and_multi_file_with_zero_duplicates()
-> TestResult {
    use super::limit_tests::{many_functions, multi_file_request, page_request};

    let limits = ResourceLimits {
        max_structure_matches: 2,
        ..ResourceLimits::default()
    };

    let single = activated_project("lib.rs", many_functions(5).as_bytes())?;
    let (single_pages, single_seen) = collect_capped_pages(
        &limits,
        |cursor| page_request(&single, cursor, None),
        |m| m.text.clone(),
    )?;
    ensure_eq(&single_pages, &3, "5 matches at cap 2 take 3 pages")?;
    ensure_eq(
        &single_seen,
        &vec!["f00", "f01", "f02", "f03", "f04"],
        "single-file pages return every match in order with zero duplicates",
    )?;

    let multi = activated_project("a.rs", b"pub fn a1() {}\npub fn a2() {}\npub fn a3() {}\n")?;
    std::fs::write(
        multi.path().join("b.rs"),
        b"pub fn b1() {}\npub fn b2() {}\n",
    )
    .check("write b.rs")?;
    let (multi_pages, multi_seen) = collect_capped_pages(
        &limits,
        |cursor| multi_file_request(&multi, cursor),
        |m| format!("{}:{}", m.file, m.text),
    )?;
    ensure_eq(
        &multi_pages,
        &3,
        "5 multi-file matches at cap 2 take 3 pages",
    )?;
    ensure_eq(
        &multi_seen,
        &vec!["a.rs:a1", "a.rs:a2", "a.rs:a3", "b.rs:b1", "b.rs:b2"],
        "multi-file pages cut mid-file and across files with zero duplicates",
    )?;
    Ok(())
}

#[test]
fn pre_file_zero_expired_time_budget_returns_structure_budget_exhausted_error() -> TestResult {
    use super::limit_tests::{multi_file_request, page_request};

    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    std::fs::write(project.path().join("b.rs"), b"pub fn beta() {}\n").check("write b.rs")?;
    let expired = ResourceLimits {
        max_structure_execution_time: std::time::Duration::ZERO,
        ..ResourceLimits::default()
    };

    for req in [
        page_request(&project, None, None),
        multi_file_request(&project, None),
    ] {
        let mut req = req;
        if req.0.file.is_some() {
            req.0.file = Some("a.rs".to_owned());
        }
        let err = structure_with_limits(
            &req,
            &expired,
            usize::MAX,
            &crate::progress::NoopProgressSink,
            &crate::sync::SourceSyncManager::default(),
        )
        .err()
        .check("already-expired budget before file 0 must return ErrorData")?;
        let data = err.data.check("structured budget error payload")?;
        ensure_eq(
            &data.get("code").and_then(serde_json::Value::as_str),
            &Some("structure_budget_exhausted"),
            "carries structure_budget_exhausted error code",
        )?;
    }
    Ok(())
}

#[test]
fn query_cursor_match_limit_overflow_marks_structure_completeness_partial() -> TestResult {
    use crate::tools::PageLimit;

    let mut dense = String::from("pub fn dense() {\n");
    for _ in 0..370 {
        dense.push_str("    let _ = 0;\n");
    }
    dense.push_str("}\n");
    let query = "(block (let_declaration) @hit (let_declaration) @hit)";

    let project = activated_project("a_clean.rs", b"pub fn clean() { let _ = 0; }\n")?;
    std::fs::write(project.path().join("b_dense.rs"), dense.as_bytes())
        .check("write b_dense.rs")?;

    let single_resp =
        ask(&project, "b_dense.rs", query).check("single-file overflow returns partial")?;
    ensure(
        !single_resp.page.complete,
        "overflowing single-file scan must not report complete",
    )?;
    ensure_eq(
        &single_resp.page.completeness,
        &Some(crate::tools::Completeness::Partial {
            limited_by: PageLimit::QueryMatchLimit,
            resume_after_file: None,
            files_scanned: 0,
            files_total: 1,
        }),
        "single-file match-limit overflow reports QueryMatchLimit partial completeness",
    )?;

    let multi_resp = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            language: Some("rust".to_owned()),
            query: query.to_owned(),
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(r)| r)
    .check("multi-file overflow returns partial")?;
    ensure(
        !multi_resp.page.complete,
        "overflowing multi-file scan must not report complete",
    )?;
    ensure_eq(
        &multi_resp.page.completeness,
        &Some(crate::tools::Completeness::Partial {
            limited_by: PageLimit::QueryMatchLimit,
            resume_after_file: Some("b_dense.rs".to_owned()),
            files_scanned: 1,
            files_total: 2,
        }),
        "multi-file match-limit overflow reports QueryMatchLimit and advances past b_dense.rs",
    )?;
    ensure_eq(
        &multi_resp.next_start_after_file.as_deref(),
        &Some("b_dense.rs"),
        "next_start_after_file advances past the overflowing file",
    )?;
    Ok(())
}
