use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

fn ask(
    project: &tempfile::TempDir,
    file: &str,
    query: &str,
) -> Result<StructureResponse, ErrorData> {
    structure(&Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: file.to_owned(),
        query: query.to_owned(),
    }))
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
            file: "lib.rs".to_owned(),
            query: "(function_item name: (identifier) @name)".to_owned(),
        }),
        &limits,
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
            file: "lib.rs".to_owned(),
            query: "(function_item name: (identifier) @name)".to_owned(),
        }),
        &limits,
    )
    .map(|Json(response)| response);
    let error = result.expect_err("runaway query is aborted, not run to completion");
    assert!(
        error.message.contains("execution time budget"),
        "unexpected message: {}",
        error.message
    );
}
