//! `query` refuses a PRAGMA wherever it sits in the input, before anything
//! is prepared, and does not mistake PRAGMA text inside a literal for one.
use super::safety_tests::{ask, fresh_pragma, project, refused};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

/// `prepare` also prepares the text after the first statement, so a PRAGMA
/// that follows a SELECT must be refused before the first prepare, not after.
#[test]
fn a_prepare_time_pragma_after_another_statement_is_refused_unapplied() -> TestResult {
    let project = project()?;
    let before = fresh_pragma("hard_heap_limit")?;

    for sql in [
        "SELECT 1; PRAGMA hard_heap_limit=1024",
        "SELECT 1;\n-- note\n  pragma hard_heap_limit=1024",
        "SELECT 1; /* c */ PRAGMA hard_heap_limit=1024;",
        "SELECT ';'; PRAGMA hard_heap_limit=1024",
        "SELECT 1; SELECT 2; PRAGMA hard_heap_limit=1024",
    ] {
        let error = refused(&project, sql, "a PRAGMA after another statement")?;
        ensure_eq(&error.code, &rmcp::model::ErrorCode::INVALID_PARAMS, sql)?;
        ensure(
            error.message.contains("not PRAGMA"),
            format!("{sql}: unexpected message: {}", error.message),
        )?;
        ensure_eq(
            &fresh_pragma("hard_heap_limit")?,
            &before,
            format!("{sql} took effect"),
        )?;
    }
    Ok(())
}

/// A `;` inside a literal, quoted identifier or comment does not start a
/// statement, so text that merely mentions `PRAGMA` after it is still a
/// single SELECT.
#[test]
fn pragma_text_after_a_semicolon_inside_a_literal_is_not_refused() -> TestResult {
    let project = project()?;

    let response = ask(
        &project,
        "SELECT '; PRAGMA x' AS a, \"b; PRAGMA\" AS c FROM (SELECT 1 AS \"b; PRAGMA\") /* ; PRAGMA y */",
    )
    .check("a literal that mentions PRAGMA")?;

    ensure_eq(&response.rows.len(), &1, "response.rows.len() == 1")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("a")
            .check("response.rows[0][\"a\"]")?,
        &"; PRAGMA x",
        "response.rows[0][\"a\"] == \"; PRAGMA x\"",
    )?;
    Ok(())
}
