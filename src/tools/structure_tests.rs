use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    let target = project.path().join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(target, content).expect("write fixture file");
    project
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
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn matches_a_real_structural_pattern() {
    let project = activated_project("lib.rs", b"pub fn greet() {}\npub fn farewell() {}\n");
    let response = ask(
        &project,
        "lib.rs",
        "(function_item name: (identifier) @name)",
    )
    .expect("query succeeds");

    assert_eq!(response.language, "rust");
    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, vec!["greet", "farewell"]);
    assert!(response.matches.iter().all(|m| m.capture_name == "name"));
    assert!(!response.truncated);
}

/// Regression: a bare node pattern with no `@capture` compiles and matches in
/// tree-sitter but yields zero captures, so the tool would silently return an
/// empty list — the exact footgun that made an agent believe `structure`
/// couldn't parse Python. It must be rejected with an actionable message
/// telling the agent to add a capture, not silently return `[]`.
#[test]
fn a_query_with_no_capture_is_rejected_with_guidance_not_empty() {
    let project = activated_project("server.py", b"def a():\n    pass\ndef b():\n    pass\n");
    let result = ask(&project, "server.py", "(function_definition)");
    let error = result.expect_err("a captureless query must error, not return empty");
    assert!(
        error.message.contains("no @capture") && error.message.contains("@name"),
        "the error must guide the agent to add a capture, got: {}",
        error.message
    );
}

/// Mirrors the live repro: a Python function query with a `@name` capture
/// returns the function definitions (proving Python parsing works once a
/// capture is present).
#[test]
fn python_function_definition_with_a_capture_returns_the_functions() {
    let project = activated_project("server.py", b"def a():\n    pass\ndef b():\n    pass\n");
    let response = ask(
        &project,
        "server.py",
        "(function_definition name: (identifier) @name)",
    )
    .expect("query with a capture succeeds");
    assert_eq!(response.language, "python");
    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
    assert!(response.matches.iter().all(|m| m.capture_name == "name"));
}

#[test]
fn an_invalid_query_is_a_typed_error_not_a_panic() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let result = ask(
        &project,
        "lib.rs",
        "(this is not valid tree-sitter query syntax",
    );
    assert!(result.is_err());
}

#[test]
fn a_missing_file_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let result = ask(&project, "does_not_exist.rs", "(function_item) @f");
    assert!(result.is_err());
}

#[test]
fn matches_against_unicode_source_without_panicking() {
    let project = activated_project(
        "lib.rs",
        "pub fn caf\u{e9}() { let s = \"\u{2603}\u{2603}\u{2603}\"; }\n".as_bytes(),
    );
    let response = ask(
        &project,
        "lib.rs",
        "(function_item name: (identifier) @name)",
    )
    .expect("query succeeds");
    assert_eq!(response.matches[0].text, "caf\u{e9}");
}

/// `start_column`/`end_column` must count *characters* since the start of
/// the line, not bytes. A multi-byte UTF-8 character earlier on the same
/// line (`é`, 2 bytes but 1 character) must not shift the column of
/// everything after it — Tree-sitter's own `Point::column` is byte-based,
/// so this pins the conversion rather than a raw pass-through.
#[test]
fn columns_count_characters_not_bytes_on_a_line_with_multibyte_utf8() {
    let project = activated_project("lib.rs", "let x = \"caf\u{e9}\"; let bar = 1;\n".as_bytes());
    let response = ask(&project, "lib.rs", "(identifier) @name").expect("query succeeds");
    let bar = response
        .matches
        .iter()
        .find(|m| m.text == "bar")
        .expect("bar identifier is matched");
    assert_eq!(
        bar.start_column, 20,
        "column must count the 1-character é, not its 2 UTF-8 bytes"
    );
}

#[test]
fn an_oversized_query_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let huge_query = "x".repeat(ResourceLimits::default().max_structure_query_bytes + 1);
    let result = ask(&project, "lib.rs", &huge_query);
    assert!(result.is_err());
}

#[test]
fn matches_are_capped_and_truncation_is_reported() {
    let project = activated_project(
        "lib.rs",
        b"pub fn a() {}\npub fn b() {}\npub fn c() {}\npub fn d() {}\n",
    );
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
        }),
        &limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
    .expect("query succeeds even though it must truncate");
    assert_eq!(response.matches.len(), 2);
    assert!(response.truncated);
}

#[test]
fn a_pathological_query_is_aborted_by_the_execution_time_budget() {
    use std::fmt::Write as _;
    let mut content = String::new();
    for index in 0..200 {
        writeln!(content, "pub fn f{index}() {{}}").expect("write to a String never fails");
    }
    let project = activated_project("lib.rs", content.as_bytes());
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
        }),
        &limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response);
    let error = result.expect_err("runaway query is aborted, not run to completion");
    assert!(
        error.message.contains("execution time budget"),
        "unexpected message: {}",
        error.message
    );
}

#[test]
fn matches_across_multiple_files_with_language_and_lean_snippets() {
    let project = activated_project("src/a.rs", b"pub fn alpha() {}\n");
    fs::write(project.path().join("src/b.rs"), b"pub fn beta() {}\n").expect("write b.rs");
    fs::write(project.path().join("src/c.py"), b"def gamma(): pass\n").expect("write c.py");

    let response = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: None,
            language: Some("rust".to_owned()),
            pattern: None,
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(resp)| resp)
    .expect("multi-file query succeeds");

    assert_eq!(response.language, "rust");
    assert_eq!(response.matches.len(), 2);
    assert_eq!(response.matches[0].file, "src/a.rs");
    assert_eq!(response.matches[0].text, "alpha");
    assert_eq!(response.matches[1].file, "src/b.rs");
    assert_eq!(response.matches[1].text, "beta");
}

#[test]
fn pattern_filters_files_in_multi_file_query() {
    let project = activated_project("src/auth/jwt.rs", b"pub fn verify() {}\n");
    fs::create_dir_all(project.path().join("src/db")).expect("mkdir db");
    fs::write(
        project.path().join("src/db/pool.rs"),
        b"pub fn connect() {}\n",
    )
    .expect("write pool.rs");

    let response = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: None,
            language: Some("rust".to_owned()),
            pattern: Some("*auth*".to_owned()),
            query: "(function_item name: (identifier) @name)".to_owned(),
            full_text: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(resp)| resp)
    .expect("filtered query succeeds");

    assert_eq!(response.matches.len(), 1);
    assert_eq!(response.matches[0].file, "src/auth/jwt.rs");
    assert_eq!(response.matches[0].text, "verify");
}
