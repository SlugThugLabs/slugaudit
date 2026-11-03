//! Capture-name, UTF-8 and query-size limits of the structure tool.
use super::*;

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
    let lines = crate::model::LineIndex::new(source);

    let (found, logs) = crate::store::test_capture::capture_warns(|| {
        crate::tools::structure_page::node_to_match(
            "lib.rs",
            tree.root_node(),
            source,
            &lines,
            None,
            false,
        )
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
    let lines = crate::model::LineIndex::new(content);

    let (found, logs) = crate::store::test_capture::capture_warns(|| {
        crate::tools::structure_page::node_to_match(
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
