//! A project-wide `structure` call checks its language against the index
//! before the pack is asked for a grammar: a language no indexed file has is
//! `unknown_language`, naming the languages this project does hold.
use super::*;
use crate::error_codes::ErrorCode;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

fn project_with(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    for (relative, content) in files {
        fs::write(project.path().join(relative), content).expect("write fixture file");
    }
    project
}

fn ask_language(
    project: &tempfile::TempDir,
    language: &str,
    query: &str,
) -> Result<StructureResponse, ErrorData> {
    structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            language: Some(language.to_owned()),
            query: query.to_owned(),
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn a_language_the_pack_does_not_know_is_unknown_language_naming_the_indexed_ones() {
    let project = project_with(&[
        ("lib.rs", b"pub fn a() {}\n"),
        ("tool.py", b"def b():\n    return 1\n"),
        ("NOTES.md", b"# notes\n"),
    ]);

    let error = ask_language(&project, "klingon", "(identifier) @name")
        .expect_err("an unknown language is refused");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error
            .message
            .ends_with("languages indexed here: python, rust"),
        "the hint lists the parsed languages in the index, prose excluded: {}",
        error.message
    );
    assert!(
        !error.message.contains("klingon"),
        "the requested name is not echoed: {}",
        error.message
    );
    let data = error.data.expect("the refusal carries data");
    assert_eq!(data["code"], "unknown_language");
    assert_eq!(data["remedy"], ErrorCode::UnknownLanguage.remedy());
}

#[test]
fn a_real_grammar_absent_from_the_project_is_unknown_language_not_a_query_error() {
    let project = project_with(&[("lib.rs", b"pub fn a() {}\n")]);

    let error = ask_language(&project, "python", "(function_item) @f")
        .expect_err("a language with no indexed file is refused");

    assert!(
        !error.message.contains("Invalid node type"),
        "the pack's query error must not stand in for the missing language: {}",
        error.message
    );
    assert!(
        error.message.ends_with("languages indexed here: rust"),
        "{}",
        error.message
    );
    assert_eq!(
        error.data.expect("the refusal carries data")["code"],
        "unknown_language"
    );
}

#[test]
fn an_indexed_language_still_runs_its_project_wide_query() {
    let project = project_with(&[("lib.rs", b"pub fn a() {}\n")]);

    let response = ask_language(&project, "rust", "(function_item name: (identifier) @name)")
        .expect("an indexed language runs");

    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(names, ["a"]);
}
