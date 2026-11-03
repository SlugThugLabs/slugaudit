//! `mechanic`: the refused adds and removes.
use super::*;

#[test]
fn adding_the_id_of_a_builtin_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(
        &manager,
        add(&path, "comment_markers", "python", EVAL_QUERY),
    )?;

    assert_refused(&error, "can never remove, rename or override a built-in")?;
    Ok(())
}

#[test]
fn adding_an_id_already_registered_for_the_session_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("first add")?;

    let error = refusal(&manager, add(&path, "py_eval", "python", EVAL_QUERY))?;

    assert_refused(&error, "already registered for this session")?;
    Ok(())
}

#[test]
fn a_query_that_does_not_compile_is_refused_with_the_compiler_message() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "broken", "python", "(call @callee"))?;

    assert_refused(&error, "query does not compile")?;
    Ok(())
}

#[test]
fn a_query_with_two_captures_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    let query = "(call function: (identifier) @callee arguments: (argument_list) @args)";

    let error = refusal(&manager, add(&path, "two", "python", query))?;

    assert_refused(&error, "exactly one capture; it names 2")?;
    Ok(())
}

#[test]
fn a_query_with_no_capture_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "none", "python", "(call)"))?;

    assert_refused(&error, "exactly one capture; it names 0")?;
    Ok(())
}

#[test]
fn a_mechanic_on_a_prose_language_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "docs", "markdown", "(paragraph) @p"))?;

    assert_refused(&error, "is prose")?;
    Ok(())
}

#[test]
fn a_mechanic_on_an_unknown_language_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "nope", "no_such_language", "(x) @x"))?;

    assert_refused(&error, "unknown language `no_such_language`")?;
    Ok(())
}

#[test]
fn an_id_outside_the_charset_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "Py-Eval", "python", EVAL_QUERY))?;

    assert_refused(&error, "a-z, 0-9 and _")?;
    Ok(())
}

#[test]
fn removing_a_builtin_is_refused() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(
        &manager,
        action(&path, MechanicAction::Remove, Some("diagnostics")),
    )?;

    assert_refused(&error, "built-ins can never be removed")?;
    Ok(())
}
