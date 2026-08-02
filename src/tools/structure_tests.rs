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
