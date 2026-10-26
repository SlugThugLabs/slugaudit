//! Tests for the `structure` tool's limit and edge branches: the empty
//! query rejection, a file with no indexed content (binary), and the
//! long-match-text truncation path. Kept separate from `structure_tests.rs`
//! so each file stays under the source-size cap.

use super::*;
use crate::tools::PageLimit;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;

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
fn an_empty_query_is_a_typed_error_not_a_silent_noop() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let result = ask(&project, "lib.rs", "   \n  ");
    let error = result.expect_err("a whitespace-only query must be rejected");
    assert!(error.message.contains("must not be empty"));
}

#[test]
fn a_file_without_indexed_content_is_a_typed_error() {
    let project = activated_project("logo.bin", b"\x89PNG\x00\x01\x02\x00\x00");
    let result = ask(&project, "logo.bin", "(identifier) @x");
    let error =
        result.expect_err("a binary file with no indexed source content must not be queried");
    assert!(
        error.message.contains("no indexed source content") || error.message.contains("logo.bin"),
        "unexpected error message: {}",
        error.message
    );
}

/// Match text longer than `MAX_TEXT_BYTES` must be truncated at a UTF-8
/// boundary and flagged with `text_truncated`, never emitted in full and
/// never panicking on the boundary slice.
#[test]
fn long_match_text_is_truncated_and_flagged() {
    let mut content = String::with_capacity(1_100_000);
    content.push_str("let long = \"");
    content.push_str(&"abcdefghij".repeat(105_000));
    content.push_str("\";\n");
    let project = activated_project("lib.rs", content.as_bytes());

    let response = ask(&project, "lib.rs", "(string_literal) @s").expect("query succeeds");
    let first = &response.matches[0];
    assert!(
        first.text_truncated,
        "a >1MB match must be flagged as truncated"
    );
    assert!(
        first.text.len() <= super::MAX_TEXT_BYTES,
        "truncated text must respect the byte cap"
    );
    assert!(
        first.text.is_char_boundary(first.text.len()),
        "truncation must land on a UTF-8 boundary"
    );
}

fn many_functions(count: usize) -> String {
    (0..count)
        .map(|index| format!("pub fn f{index:02}() {{}}\n"))
        .collect()
}

fn page_request(
    project: &tempfile::TempDir,
    offset: usize,
    page_bytes: Option<usize>,
) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: Some("lib.rs".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        offset,
        page_bytes,
        ..Default::default()
    })
}

fn run_with(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    window_bytes: usize,
) -> StructureResponse {
    structure_with_limits(
        request,
        limits,
        window_bytes,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
    .expect("structure call succeeds")
}

#[test]
fn a_page_cut_by_bytes_continues_through_next_offset_to_every_match() {
    let project = activated_project("lib.rs", many_functions(30).as_bytes());
    let limits = ResourceLimits::default();
    let mut names = Vec::new();
    let mut offset = 0;
    let mut pages = 0;

    loop {
        let response = run_with(
            &page_request(&project, offset, Some(900)),
            &limits,
            usize::MAX,
        );
        pages += 1;
        names.extend(response.matches.iter().map(|m| m.text.clone()));
        assert_eq!(!response.page.complete, response.page.next_offset.is_some());
        assert_eq!(response.page.returned, response.matches.len());
        assert_eq!(response.page.total, None, "structure never counts ahead");
        match response.page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }

    let expected: Vec<String> = (0..30).map(|index| format!("f{index:02}")).collect();
    assert_eq!(names, expected);
    assert!(pages > 1, "a 900-byte page must not hold 30 matches");
}

#[test]
fn a_page_bytes_override_changes_the_page_and_is_echoed() {
    let project = activated_project("lib.rs", many_functions(30).as_bytes());
    let limits = ResourceLimits::default();

    let small = run_with(&page_request(&project, 0, Some(900)), &limits, usize::MAX);
    let default = run_with(&page_request(&project, 0, None), &limits, usize::MAX);

    assert_eq!(small.page.page_bytes, 900);
    assert_eq!(default.page.page_bytes, limits.default_page_bytes);
    assert!(small.matches.len() < default.matches.len());
    assert_eq!(default.matches.len(), 30);
    assert!(default.page.complete && default.page.next_offset.is_none());
    assert_eq!(default.page.limited_by, PageLimit::None);
    assert_eq!(small.page.limited_by, PageLimit::PageBytes);
    assert!(small.page.bytes_used <= 900);
}

#[test]
fn a_page_bytes_above_the_ceiling_is_clamped_and_a_cut_reports_the_ceiling() {
    let project = activated_project("lib.rs", many_functions(100).as_bytes());
    let limits = ResourceLimits {
        max_query_response_bytes: 4_096,
        ..ResourceLimits::default()
    };

    let response = run_with(
        &page_request(&project, 0, Some(10_000_000)),
        &limits,
        usize::MAX,
    );

    assert_eq!(response.page.page_bytes, 4_096);
    assert_eq!(response.page.limited_by, PageLimit::Ceiling);
    assert!(!response.page.complete);
}

#[test]
fn a_match_count_exactly_at_the_cap_is_not_reported_as_cut() {
    let project = activated_project("lib.rs", many_functions(4).as_bytes());
    let limits = ResourceLimits {
        max_structure_matches: 4,
        ..ResourceLimits::default()
    };

    let response = run_with(&page_request(&project, 0, None), &limits, usize::MAX);

    assert_eq!(response.matches.len(), 4);
    assert!(response.page.complete && response.page.next_offset.is_none());
    assert_eq!(response.page.limited_by, PageLimit::None);
}

#[test]
fn a_zero_page_bytes_is_refused_rather_than_substituted() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");

    let result = structure(
        &page_request(&project, 0, Some(0)),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    assert!(result.is_err());
}

#[test]
fn start_after_file_with_a_single_file_query_is_refused() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = page_request(&project, 0, None);
    request.0.start_after_file = Some("a.rs".to_owned());

    let result = structure(
        &request,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    let error = result.err().expect("start_after_file needs multi-file");
    assert!(
        error.message.contains("start_after_file"),
        "{}",
        error.message
    );
}

#[test]
fn files_past_the_memory_window_are_reported_and_a_continuation_reaches_them() {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    for (name, body) in [
        ("b.rs", "pub fn beta() {}\n"),
        ("c.rs", "pub fn gamma() {}\n"),
    ] {
        std::fs::write(project.path().join(name), body).expect("write fixture");
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
        let response = run_with(&request, &limits, 18);
        windows += 1;
        names.extend(response.matches.iter().map(|m| m.text.clone()));
        match response.next_start_after_file {
            Some(next) => {
                assert!(response.files_not_scanned > 0 && response.bytes_not_scanned > 0);
                assert!(!response.page.complete, "a window remains");
                assert_eq!(response.page.limited_by, PageLimit::ScanWindow);
                assert_eq!(response.page.next_offset, None);
                start_after = Some(next);
            }
            None => {
                assert_eq!(response.files_not_scanned, 0);
                assert_eq!(response.bytes_not_scanned, 0);
                assert!(response.page.complete);
                break;
            }
        }
    }

    assert_eq!(names, ["alpha", "beta", "gamma"]);
    assert_eq!(windows, 3);
}

#[test]
fn a_window_that_ends_exactly_at_the_last_file_reports_nothing_unscanned() {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    let request = Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        ..Default::default()
    });

    // "pub fn alpha() {}\n" is 18 bytes: the window ends exactly at it.
    let response = run_with(&request, &ResourceLimits::default(), 18);

    assert_eq!(response.files_not_scanned, 0);
    assert_eq!(response.next_start_after_file, None);
    assert_eq!(response.matches.len(), 1);
}

fn multi_file_request(project: &tempfile::TempDir, offset: usize) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        offset,
        ..Default::default()
    })
}

/// A page cut inside a multi-file scan window leaves `files_not_scanned` at
/// 0 even though later loaded files were not scanned by this call. The cut is
/// signaled by `page.next_offset` and `page.limited_by` — `files_not_scanned` only counts
/// files the memory window left unloaded — and the next page resumes into the
/// later file.
#[test]
fn a_page_cut_inside_a_multi_file_window_is_signaled_by_next_offset_not_files_not_scanned() {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    std::fs::write(project.path().join("b.rs"), b"pub fn beta() {}\n").expect("write b.rs");
    let limits = ResourceLimits {
        max_structure_matches: 1,
        ..ResourceLimits::default()
    };

    let first = run_with(&multi_file_request(&project, 0), &limits, usize::MAX);

    assert_eq!(
        first.page.limited_by,
        PageLimit::MatchCap,
        "the page stopped at the match cap inside a.rs"
    );
    assert_eq!(first.page.next_offset, Some(1));
    assert_eq!(first.files_not_scanned, 0);
    assert_eq!(first.next_start_after_file, None);
    assert_eq!(first.matches.len(), 1);
    assert_eq!(first.matches[0].file, "a.rs");

    let second = run_with(&multi_file_request(&project, 1), &limits, usize::MAX);

    assert!(second.page.complete);
    assert_eq!(second.matches.len(), 1);
    assert_eq!(second.matches[0].file, "b.rs");
    assert_eq!(second.matches[0].text, "beta");
}

fn single_file_request(project: &tempfile::TempDir) -> StructureRequest {
    StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: Some("lib.rs".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        ..Default::default()
    }
}

fn refusal(request: StructureRequest) -> ErrorData {
    structure(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("the request must be refused")
}

#[test]
fn a_path_over_the_cap_is_refused_before_any_sync() {
    let mut request = single_file_request(&activated_project("lib.rs", b"pub fn a() {}\n"));
    request.path = "x".repeat(32 * 1024 + 1);

    let error = structure_with_limits(
        &Parameters(request),
        &ResourceLimits::default(),
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("an oversized path must be refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message.as_ref(), "path exceeds 32768 bytes");
}

#[test]
fn an_oversized_start_after_file_is_refused_with_its_cap() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = single_file_request(&project);
    request.file = None;
    request.language = Some("rust".to_owned());
    request.start_after_file = Some("x".repeat(32 * 1024 + 1));

    let error = refusal(request);

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "start_after_file exceeds 32768 bytes"
    );
}

#[test]
fn an_oversized_expected_revision_is_refused_with_its_cap() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = single_file_request(&project);
    request.expected_revision = Some("x".repeat(257));

    let error = refusal(request);

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "expected_revision exceeds 256 bytes"
    );
}

#[test]
fn an_oversized_file_filter_is_refused_by_name() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = single_file_request(&project);
    request.file = Some("x".repeat(8 * 1024 + 1));

    let error = refusal(request);

    assert_eq!(error.message.as_ref(), "file exceeds 8192 bytes");
}

#[test]
fn an_oversized_language_filter_is_refused_by_name() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = single_file_request(&project);
    request.file = None;
    request.language = Some("x".repeat(8 * 1024 + 1));

    let error = refusal(request);

    assert_eq!(error.message.as_ref(), "language exceeds 8192 bytes");
}

#[test]
fn an_oversized_pattern_filter_is_refused_by_name() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = single_file_request(&project);
    request.pattern = Some("x".repeat(8 * 1024 + 1));

    let error = refusal(request);

    assert_eq!(error.message.as_ref(), "pattern exceeds 8192 bytes");
}

#[test]
fn an_unrepresentable_execution_budget_means_no_deadline_not_a_panic() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let limits = ResourceLimits {
        max_structure_execution_time: std::time::Duration::MAX,
        ..ResourceLimits::default()
    };

    let response = run_with(&page_request(&project, 0, None), &limits, usize::MAX);

    assert_eq!(response.matches.len(), 1);
}

fn rust_root(source: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language_pack::get_language("rust").expect("rust grammar"))
        .expect("set language");
    parser.parse(source, None).expect("parse")
}

#[test]
fn a_capture_without_a_name_is_logged_and_flagged_as_failed_extraction() {
    let source = "fn a() {}";
    let tree = rust_root(source);
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
    });

    assert!(found.extraction_failed);
    assert!(logs.contains("capture has no name"), "logs: {logs}");
}

#[test]
fn text_that_is_not_valid_utf8_keeps_its_cause_in_the_log() {
    let tree = rust_root("abc");
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
    });

    assert!(found.extraction_failed);
    assert!(found.text.is_empty());
    assert!(logs.contains("not valid UTF-8"), "logs: {logs}");
    assert!(logs.contains("utf-8"), "source error kept: {logs}");
}

fn query_limit(bytes: usize) -> ResourceLimits {
    ResourceLimits {
        max_structure_query_bytes: bytes,
        ..ResourceLimits::default()
    }
}

#[test]
fn a_query_one_byte_over_the_limit_is_refused_by_size_before_anything_else_runs() {
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
    .expect_err("33 bytes is over a 32-byte limit");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message,
        "structure query is 33 bytes; the limit is 32 (max_structure_query_bytes)"
    );
}

#[test]
fn a_query_of_exactly_the_limit_is_not_refused_for_size() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let query = format!("(identifier) @name{}", " ".repeat(14));
    assert_eq!(query.len(), 32);

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
    .expect("a query at the limit compiles and runs");

    assert_eq!(response.matches.len(), 1);
}

#[test]
fn a_project_config_query_limit_is_honoured_by_the_tool() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let config_dir = project.path().join(".planning").join("slugaudit");
    std::fs::write(
        config_dir.join("config.json"),
        r#"{"max_structure_query_bytes": 20}"#,
    )
    .expect("write config");

    let error = ask(
        &project,
        "lib.rs",
        "(function_item name: (identifier) @name)",
    )
    .expect_err("the 40-byte query is over the configured 20");

    assert_eq!(
        error.message,
        "structure query is 40 bytes; the limit is 20 (max_structure_query_bytes)"
    );
}
