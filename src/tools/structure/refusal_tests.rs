//! Structure requests refused for a missing file or language filter, or for prose.
use super::tests::{activated_project, ask};
use super::*;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

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
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"structure request without 'file' must specify 'language'",
        "error.message.as_ref() == \"structure request without 'file' must specify 'language'\"",
    )?;
    Ok(())
}

fn assert_prose_refusal(error: &ErrorData) -> TestResult {
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    let message = error.message.to_lowercase();
    ensure(message.contains("not parsed"), format!("{}", error.message))?;
    ensure(!message.contains("download"), format!("{}", error.message))?;
    ensure(!message.contains("manifest"), format!("{}", error.message))?;
    Ok(())
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
    assert_prose_refusal(&error)?;
    Ok(())
}

#[test]
fn structure_on_a_language_shadowed_by_prose_is_refused_as_not_parsed() -> TestResult {
    let project = activated_project("a.md", b"# title\n")?;
    let error = ask_language(&project, "markdown").check_err("markdown is stored as prose")?;
    assert_prose_refusal(&error)?;
    Ok(())
}

#[test]
fn structure_on_a_prose_file_is_refused_as_not_parsed() -> TestResult {
    let project = activated_project("a.md", b"# title\n")?;
    let error = ask(&project, "a.md", "(document) @d").check_err("a.md is stored as prose")?;
    assert_prose_refusal(&error)?;
    Ok(())
}
