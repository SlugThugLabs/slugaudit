use super::*;
use crate::tools::PageLimit;
use crate::tools::context::ensure_synced;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::fs;

fn activated_project(relative: &str, content: &[u8]) -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    let target = project.path().join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).check("create parent")?;
    }
    fs::write(target, content).check("write fixture file")?;
    Ok(project)
}

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
fn matches_a_real_structural_pattern() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn greet() {}\npub fn farewell() {}\n")?;
    let response = ask(
        &project,
        "lib.rs",
        "(function_item name: (identifier) @name)",
    )
    .check("query succeeds")?;

    assert_eq!(response.language, "rust");
    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, vec!["greet", "farewell"]);
    assert!(response.matches.iter().all(|m| m.capture_name == "name"));
    assert!(response.page.complete);
    Ok(())
}

/// A bare node pattern with no `@capture` compiles and matches in
/// tree-sitter but yields zero captures, so the tool would silently return an
/// empty list, which reads as `structure`
/// being unable to parse the language. It must be rejected with an actionable message
/// telling the agent to add a capture, not silently return `[]`.
#[test]
fn a_query_with_no_capture_is_rejected_with_guidance_not_empty() -> TestResult {
    let project = activated_project("server.py", b"def a():\n    pass\ndef b():\n    pass\n")?;
    let result = ask(&project, "server.py", "(function_definition)");
    let error = result.check_err("a captureless query must error, not return empty")?;
    assert!(
        error.message.contains("no @capture") && error.message.contains("@name"),
        "the error must guide the agent to add a capture, got: {}",
        error.message
    );
    Ok(())
}

/// A Python function query with a `@name` capture
/// returns the function definitions (proving Python parsing works once a
/// capture is present).
#[test]
fn python_function_definition_with_a_capture_returns_the_functions() -> TestResult {
    let project = activated_project("server.py", b"def a():\n    pass\ndef b():\n    pass\n")?;
    let response = ask(
        &project,
        "server.py",
        "(function_definition name: (identifier) @name)",
    )
    .check("query with a capture succeeds")?;
    assert_eq!(response.language, "python");
    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
    assert!(response.matches.iter().all(|m| m.capture_name == "name"));
    Ok(())
}

#[test]
fn an_invalid_query_is_a_typed_error_not_a_panic() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let result = ask(
        &project,
        "lib.rs",
        "(this is not valid tree-sitter query syntax",
    );
    let error = result.check_err("invalid query must return ErrorData")?;
    assert!(
        error.message.to_lowercase().contains("syntax")
            || error.message.to_lowercase().contains("query"),
        "unexpected error message: {}",
        error.message
    );
    Ok(())
}

/// A file absent from the index is the caller's input being wrong, and the
/// error must say *which* condition with the machine-readable code — not
/// merely echo the filename, which is true of every error on this path.
#[test]
fn a_missing_file_is_refused_with_the_file_not_indexed_code() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;

    let error = ask(&project, "does_not_exist.rs", "(function_item) @f")
        .check_err("missing file must return ErrorData")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    let data = error.data.check("a missing file carries a payload")?;
    assert_eq!(data["code"], "file_not_indexed", "{}", error.message);
    assert_eq!(
        error.message.as_ref(),
        "does_not_exist.rs is not indexed in the current revision"
    );
    Ok(())
}

#[test]
fn matches_against_unicode_source_without_panicking() -> TestResult {
    let project = activated_project(
        "lib.rs",
        "pub fn caf\u{e9}() { let s = \"\u{2603}\u{2603}\u{2603}\"; }\n".as_bytes(),
    )?;
    let response = ask(
        &project,
        "lib.rs",
        "(function_item name: (identifier) @name)",
    )
    .check("query succeeds")?;
    assert_eq!(response.matches[0].text, "caf\u{e9}");
    Ok(())
}

/// `start_column`/`end_column` must count *characters* since the start of
/// the line, not bytes. A multi-byte UTF-8 character earlier on the same
/// line (`é`, 2 bytes but 1 character) must not shift the column of
/// everything after it — Tree-sitter's own `Point::column` is byte-based,
/// so this pins the conversion rather than a raw pass-through.
#[test]
fn columns_count_characters_not_bytes_on_a_line_with_multibyte_utf8() -> TestResult {
    let project = activated_project("lib.rs", "let x = \"caf\u{e9}\"; let bar = 1;\n".as_bytes())?;
    let response = ask(&project, "lib.rs", "(identifier) @name").check("query succeeds")?;
    let bar = response
        .matches
        .iter()
        .find(|m| m.text == "bar")
        .check("bar identifier is matched")?;
    assert_eq!(
        bar.start_column, 20,
        "column must count the 1-character é, not its 2 UTF-8 bytes"
    );
    Ok(())
}

#[test]
fn an_oversized_query_is_a_typed_error() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let huge_query = "x".repeat(ResourceLimits::default().max_structure_query_bytes + 1);
    let result = ask(&project, "lib.rs", &huge_query);
    let error = result.check_err("oversized query must return ErrorData")?;
    assert!(
        error.message.to_lowercase().contains("bytes")
            || error.message.to_lowercase().contains("large")
            || error.message.to_lowercase().contains("exceed"),
        "unexpected error message: {}",
        error.message
    );
    Ok(())
}

#[test]
fn matches_are_capped_and_truncation_is_reported() -> TestResult {
    let project = activated_project(
        "lib.rs",
        b"pub fn a() {}\npub fn b() {}\npub fn c() {}\npub fn d() {}\n",
    )?;
    let limits = ResourceLimits {
        max_structure_matches: 2,
        ..ResourceLimits::default()
    };
    let response = structure_with_limits(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: Some("lib.rs".to_owned()),
            language: None,
            pattern: None,
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: None,
            ..Default::default()
        }),
        &limits,
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
    .check("query succeeds even though it must truncate")?;
    assert_eq!(response.matches.len(), 2);
    assert_eq!(response.page.limited_by, PageLimit::MatchCap);
    assert!(!response.page.complete);
    Ok(())
}

#[test]
fn a_pathological_query_is_aborted_by_the_execution_time_budget() -> TestResult {
    use std::fmt::Write as _;
    let mut content = String::new();
    for index in 0..200 {
        writeln!(content, "pub fn f{index}() {{}}").check("write to a String never fails")?;
    }
    let project = activated_project("lib.rs", content.as_bytes())?;
    let limits = ResourceLimits {
        // Effectively zero: by the time the native progress callback first
        // fires (every ~100 internal query operations), any positive
        // elapsed time trips this deadline, while match/query-byte limits
        // stay at their generous defaults so neither is what catches this.
        max_structure_execution_time: std::time::Duration::from_nanos(1),
        ..ResourceLimits::default()
    };
    let result = structure_with_limits(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: Some("lib.rs".to_owned()),
            language: None,
            pattern: None,
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: None,
            ..Default::default()
        }),
        &limits,
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response);
    let error = result.check_err("runaway query is aborted, not run to completion")?;
    assert!(
        error.message.contains("execution time budget"),
        "unexpected message: {}",
        error.message
    );
    Ok(())
}

#[test]
fn matches_across_multiple_files_with_language_and_lean_snippets() -> TestResult {
    let project = activated_project("src/a.rs", b"pub fn alpha() {}\n")?;
    fs::write(project.path().join("src/b.rs"), b"pub fn beta() {}\n").check("write b.rs")?;
    fs::write(project.path().join("src/c.py"), b"def gamma(): pass\n").check("write c.py")?;

    let response = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: None,
            language: Some("rust".to_owned()),
            pattern: None,
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: None,
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(resp)| resp)
    .check("multi-file query succeeds")?;

    assert_eq!(response.language, "rust");
    assert_eq!(response.matches.len(), 2);
    assert_eq!(response.matches[0].file, "src/a.rs");
    assert_eq!(response.matches[0].text, "alpha");
    assert_eq!(response.matches[1].file, "src/b.rs");
    assert_eq!(response.matches[1].text, "beta");
    Ok(())
}

#[test]
fn pattern_filters_files_in_multi_file_query() -> TestResult {
    let project = activated_project("src/auth/jwt.rs", b"pub fn verify() {}\n")?;
    fs::create_dir_all(project.path().join("src/db")).check("mkdir db")?;
    fs::write(
        project.path().join("src/db/pool.rs"),
        b"pub fn connect() {}\n",
    )
    .check("write pool.rs")?;

    let response = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: None,
            language: Some("rust".to_owned()),
            pattern: Some("*auth*".to_owned()),
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: None,
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(resp)| resp)
    .check("filtered query succeeds")?;

    assert_eq!(response.matches.len(), 1);
    assert_eq!(response.matches[0].file, "src/auth/jwt.rs");
    assert_eq!(response.matches[0].text, "verify");
    Ok(())
}

#[test]
fn a_structure_query_against_a_binary_file_carries_the_file_not_indexed_code() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    fs::write(project.path().join("blob.dat"), b"\0\x01binary\n").check("binary file")?;

    let result = ask(&project, "blob.dat", "(function_item) @f");

    let error = result.check_err("a binary file has no indexed content")?;
    let data = error.data.check("a binary file carries a payload")?;
    assert_eq!(data["code"], "file_not_indexed", "{}", error.message);
    Ok(())
}

#[test]
fn a_file_with_no_detected_language_is_refused_by_name() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let synced = ensure_synced(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("sync")?;
    let connection = crate::store::open_read_write(&synced.database_path).check("open database")?;
    connection
        .execute("UPDATE files SET language = NULL WHERE path = 'lib.rs'", [])
        .check("clear the detected language")?;

    let error = crate::tools::structure_source::fetch_source(&connection, "lib.rs", u64::MAX)
        .check_err("a file with no detected language must be refused")?;

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message.as_ref(), "lib.rs has no detected language");
    Ok(())
}

/// A failure to read the index is operational, not the caller's input. An
/// in-memory database with no `files` table is a hermetically reproducible
/// schema-drift failure: it must be `internal_error` (with the operation
/// named and the source preserved), never `invalid_params` blaming the
/// `file` argument.
#[test]
fn a_database_failure_reading_a_file_is_an_internal_error_not_a_bad_argument() -> TestResult {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;

    let error = crate::tools::structure_source::fetch_source(&connection, "lib.rs", u64::MAX)
        .check_err("a missing files table must fail")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{error:?}"
    );
    assert!(
        error.message.contains("reading the indexed file row"),
        "the operation must be named: {}",
        error.message
    );
    assert!(
        error.message.contains("no such table: files"),
        "the underlying cause must survive: {}",
        error.message
    );
    assert!(
        error.message.contains("retry the call"),
        "the standard transient-failure hint must be present: {}",
        error.message
    );
    assert!(
        error.data.is_none(),
        "an operational failure carries no caller-input code: {:?}",
        error.data
    );
    Ok(())
}

/// The multi-file scan's database failures must also reach the caller as
/// operational errors that name the operation and carry the retry hint,
/// not as the caller's bad input.
#[test]
fn a_database_failure_loading_files_is_an_internal_error_not_an_empty_scan() -> TestResult {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;

    let error = crate::tools::structure_source::fetch_sources_for_language(
        &connection,
        "rust",
        None,
        None,
        crate::tools::structure_source::WindowBounds {
            window_bytes: MAX_MULTI_FILE_TOTAL_BYTES,
            max_parse_bytes: u64::MAX,
        },
    )
    .check_err("a missing files table must fail, not scan nothing")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{error:?}"
    );
    assert!(
        error.message.contains("preparing the indexed-file scan"),
        "the operation must be named: {}",
        error.message
    );
    assert!(
        error.message.contains("no such table: files"),
        "the underlying cause must survive: {}",
        error.message
    );
    assert!(
        error.message.contains("retry the call"),
        "the standard transient-failure hint must be present: {}",
        error.message
    );
    Ok(())
}

#[test]
fn a_multi_file_request_without_file_or_language_is_refused() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;

    let result = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            query: "(identifier) @name".to_owned(),
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    let error = result
        .err()
        .check("neither file nor language cannot select any target")?;
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "structure request without 'file' must specify 'language'"
    );
    Ok(())
}

fn assert_prose_refusal(error: &ErrorData) {
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    let message = error.message.to_lowercase();
    assert!(message.contains("not parsed"), "{}", error.message);
    assert!(!message.contains("download"), "{}", error.message);
    assert!(!message.contains("manifest"), "{}", error.message);
}

fn ask_language(
    project: &tempfile::TempDir,
    language: &str,
) -> Result<StructureResponse, ErrorData> {
    structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            language: Some(language.to_owned()),
            query: "(document) @d".to_owned(),
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn structure_on_the_prose_language_is_refused_as_not_parsed() -> TestResult {
    let project = activated_project("a.md", b"# title\n")?;
    let error = ask_language(&project, "prose").check_err("prose is not parsed")?;
    assert_prose_refusal(&error);
    Ok(())
}

#[test]
fn structure_on_a_language_shadowed_by_prose_is_refused_as_not_parsed() -> TestResult {
    let project = activated_project("a.md", b"# title\n")?;
    let error = ask_language(&project, "markdown").check_err("markdown is stored as prose")?;
    assert_prose_refusal(&error);
    Ok(())
}

#[test]
fn structure_on_a_prose_file_is_refused_as_not_parsed() -> TestResult {
    let project = activated_project("a.md", b"# title\n")?;
    let error = ask(&project, "a.md", "(document) @d").check_err("a.md is stored as prose")?;
    assert_prose_refusal(&error);
    Ok(())
}
