//! Cursor authentication (`unknown_cursor` policy) and `page_request`
//! parameter resolution for [`super::paginate`].

use super::tests::{ROOMY, TOOL, code_of, cursor_of, fresh, manager, next, project};
use super::*;
use crate::progress::NoopProgressSink;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn paginate_cursor_of_another_tool_is_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;

    let error = next(&manager, "other", &cursor_of(&first)?, 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_wrong_tool_cursor_on_the_last_page_does_not_mark_delivered() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 15, 10).check("page one")?;
    let stats = || -> TestResult<_> {
        let stats = manager
            .result_stores()
            .current()
            .check("the store slot")?
            .check("a store")?
            .stats()
            .check("read the store counters")?;
        Ok((
            stats.active.results,
            stats.idle.results,
            stats.delivered.results,
        ))
    };
    let before = stats()?;

    let error = next(&manager, "other", &cursor_of(&first)?, 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    ensure_eq(&before, &(1, 0, 0), "before == (1, 0, 0)")?;
    ensure_eq(&stats()?, &before, "stats()? == before")?;
    next(&manager, TOOL, &cursor_of(&first)?, 10).check("the right tool finishes it")?;
    ensure_eq(&stats()?, &(0, 0, 1), "stats()? == (0, 0, 1)")?;
    Ok(())
}

#[test]
fn paginate_garbage_cursor_is_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    fresh(&manager, &path, 5, 10).check("opens the store")?;

    let error = next(&manager, TOOL, "not-a-cursor", 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_cursor_from_another_process_nonce_is_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    let token = cursor_of(&first)?;
    let mut parts: Vec<&str> = token.split('.').collect();
    let foreign = uuid::Uuid::new_v4().hyphenated().to_string();
    *parts.get_mut(1).check("parts[1]")? = &foreign;

    let error = next(&manager, TOOL, &parts.join("."), 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_cursor_before_any_result_exists_is_unknown_cursor() -> TestResult {
    let manager = manager(ROOMY);
    let token = format!("v1.{0}.{0}.0", uuid::Uuid::new_v4().hyphenated());

    let error = next(&manager, TOOL, &token, 10).err().check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn page_request_with_a_cursor_continues_and_ignores_the_path() -> TestResult {
    let request = page_request(Some("tok"), Some("/ignored"), &NoopProgressSink)
        .check("a cursor resolves")?;

    ensure(
        matches!(request, PageRequest::Cursor("tok")),
        "matches!(request, PageRequest::Cursor(\"tok\"))",
    )?;
    Ok(())
}

#[test]
fn page_request_without_a_cursor_is_fresh_on_the_path() -> TestResult {
    let request =
        page_request(None, Some("/project"), &NoopProgressSink).check("a path resolves")?;

    ensure(
        matches!(
            request,
            PageRequest::Fresh {
                path: "/project",
                ..
            }
        ),
        "matches!( request, PageRequest::Fresh { path: \"/project\", .. } )",
    )?;
    Ok(())
}

#[test]
fn page_request_with_neither_cursor_nor_path_is_invalid_params() -> TestResult {
    let error = page_request(None, None, &NoopProgressSink)
        .err()
        .check("nothing to resolve")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &"path is required when no cursor is given",
        "error.message == \"path is required when no cursor is given\"",
    )?;
    Ok(())
}
