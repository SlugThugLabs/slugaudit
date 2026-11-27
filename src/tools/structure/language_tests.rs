//! A project-wide `structure` call checks its language against the index
//! before the pack is asked for a grammar: a language no indexed file has is
//! `unknown_language`, naming the languages this project does hold.
use super::*;
use crate::error_codes::ErrorCode;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

fn project_with(files: &[(&str, &[u8])]) -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    for (relative, content) in files {
        fs::write(project.path().join(relative), content).check("write fixture file")?;
    }
    Ok(project)
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
fn a_language_the_pack_does_not_know_is_unknown_language_naming_the_indexed_ones() -> TestResult {
    let project = project_with(&[
        ("lib.rs", b"pub fn a() {}\n"),
        ("tool.py", b"def b():\n    return 1\n"),
        ("NOTES.md", b"# notes\n"),
    ])?;

    let error = ask_language(&project, "klingon", "(identifier) @name")
        .check_err("an unknown language is refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        format!("{error:?}"),
    )?;
    ensure(
        error
            .message
            .ends_with("languages indexed here: python, rust"),
        format!(
            "the hint lists the parsed languages in the index, prose excluded: {}",
            error.message
        ),
    )?;
    ensure(
        !error.message.contains("klingon"),
        format!("the requested name is not echoed: {}", error.message),
    )?;
    let data = error.data.check("the refusal carries data")?;
    ensure_eq(
        &data.get("code").check("data[\"code\"]")?,
        &"unknown_language",
        "data[\"code\"] == \"unknown_language\"",
    )?;
    ensure_eq(
        &data.get("remedy").check("data[\"remedy\"]")?,
        &ErrorCode::UnknownLanguage.remedy(),
        "data[\"remedy\"] == ErrorCode::UnknownLanguage.remedy()",
    )?;
    Ok(())
}

#[test]
fn a_real_grammar_absent_from_the_project_is_unknown_language_not_a_query_error() -> TestResult {
    let project = project_with(&[("lib.rs", b"pub fn a() {}\n")])?;

    let error = ask_language(&project, "python", "(function_item) @f")
        .check_err("a language with no indexed file is refused")?;

    ensure(
        !error.message.contains("Invalid node type"),
        format!(
            "the pack's query error must not stand in for the missing language: {}",
            error.message
        ),
    )?;
    ensure(
        error.message.ends_with("languages indexed here: rust"),
        format!("{}", error.message),
    )?;
    ensure_eq(
        &error
            .data
            .check("the refusal carries data")?
            .get("code")
            .check("error.data.check(\"the refusal carries data\")?[\"code\"]")?,
        &"unknown_language",
        "error.data.check(\"the refusal carries data\")?[\"code\"] == \"unknown_language\"",
    )?;
    Ok(())
}

#[test]
fn an_indexed_language_still_runs_its_project_wide_query() -> TestResult {
    let project = project_with(&[("lib.rs", b"pub fn a() {}\n")])?;

    let response = ask_language(&project, "rust", "(function_item name: (identifier) @name)")
        .check("an indexed language runs")?;

    let names: Vec<&str> = response.matches.iter().map(|m| m.text.as_str()).collect();
    ensure_eq(&names, &["a"], "names == [\"a\"]")?;
    Ok(())
}

#[test]
fn an_untested_language_query_carries_one_notice() -> TestResult {
    let project = project_with(&[
        ("lib.rs", b"pub fn a() {}\n"),
        ("tool.py", b"def b():\n    return 1\n"),
    ])?;

    let response = ask_language(
        &project,
        "python",
        "(function_definition name: (identifier) @name)",
    )
    .check("an indexed language runs")?;

    ensure_eq(
        &response.notices,
        &[
            "python: uses an untested grammar (downloaded on first use). \
             Only Rust is tested; treat python results as best-effort.",
        ],
        "response.notices == [ \"python: uses an untested grammar (downloaded on first use). \\ Only Rust is tested; treat python results as best-effort.\" ]",
    )?;
    Ok(())
}

#[test]
fn a_rust_query_carries_no_notice() -> TestResult {
    let project = project_with(&[
        ("lib.rs", b"pub fn a() {}\n"),
        ("tool.py", b"def b():\n    return 1\n"),
    ])?;

    let response = ask_language(&project, "rust", "(function_item name: (identifier) @name)")
        .check("an indexed language runs")?;

    ensure(
        response.notices.is_empty(),
        format!("{:?}", response.notices),
    )?;
    Ok(())
}

#[test]
fn a_pack_alias_language_in_structure_is_refused_with_its_canonical_name() -> TestResult {
    let project = project_with(&[("run.sh", b"#!/bin/sh\necho hi\n")])?;

    let error = ask_language(&project, "shell", "(command) @c")
        .check_err("alias `shell` must be refused naming `bash`")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "alias language is invalid_params",
    )?;
    ensure(
        error.message.contains("use `bash`"),
        format!(
            "expected canonical hint `use `bash``, got: {}",
            error.message
        ),
    )?;
    Ok(())
}
