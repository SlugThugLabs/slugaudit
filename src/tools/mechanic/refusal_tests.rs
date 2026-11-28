//! `mechanic`: the refused adds and removes.
use super::tests::{
    EVAL_QUERY, action, add, assert_refused, call, path_of, python_project, refusal,
};
use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

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

fn padded_dense_let_fn(statement_count: usize) -> String {
    let mut source = String::from("pub fn dense() {\n");
    for _ in 0..statement_count {
        source.push_str("    let padding_identifier_for_spill_threshold = 0;\n");
    }
    source.push_str("}\n");
    source
}

#[test]
fn mechanic_add_spilling_then_failing_trial_scan_leaks_zero_result_store_bytes() -> TestResult {
    use super::scan_tests::{OVERFLOW_LET_QUERY, dense_let_fn};

    let spill_project = crate::tools::test_support::activated_project(
        "a_spill.rs",
        padded_dense_let_fn(120).as_bytes(),
    )?;
    std::fs::write(
        spill_project.path().join("z_overflow.rs"),
        dense_let_fn(370).as_bytes(),
    )
    .check("write z_overflow.rs")?;
    let spill_path = path_of(&spill_project)?;
    let spill_manager = sync::SourceSyncManager::default();
    let mut spill_req = add(&spill_path, "let_quads", "rust", OVERFLOW_LET_QUERY);
    spill_req.page_bytes = Some(1);

    let spill_err = refusal(&spill_manager, spill_req)?;
    ensure_eq(
        &spill_err
            .data
            .as_ref()
            .and_then(|d| d.get("code"))
            .and_then(serde_json::Value::as_str),
        &Some("mechanic_pattern_too_broad"),
        "spilled partial trial scan returns mechanic_pattern_too_broad",
    )?;
    let store = spill_manager
        .result_stores()
        .current()
        .check("read result store slot")?
        .check("spilled trial scan must enter SinkMode::Spilled and open ResultStore")?;
    let stats = store.stats().check("read result store stats")?;
    ensure_eq(
        &stats.admitted_bytes,
        &0,
        "spilled writer must not admit bytes",
    )?;
    let results = stats
        .active
        .results
        .checked_add(stats.idle.results)
        .and_then(|n| n.checked_add(stats.delivered.results))
        .check("sum store results")?;
    ensure_eq(
        &results,
        &0,
        "spilled writer must leave zero stored results",
    )?;

    let buf_project =
        crate::tools::test_support::activated_project("a_small.rs", dense_let_fn(2).as_bytes())?;
    std::fs::write(
        buf_project.path().join("z_overflow.rs"),
        dense_let_fn(370).as_bytes(),
    )
    .check("write z_overflow.rs")?;
    let buf_path = path_of(&buf_project)?;
    let buf_manager = sync::SourceSyncManager::default();
    let mut buf_req = add(&buf_path, "let_quads", "rust", OVERFLOW_LET_QUERY);
    buf_req.limit = Some(1);

    let buf_err = refusal(&buf_manager, buf_req)?;
    ensure_eq(
        &buf_err
            .data
            .as_ref()
            .and_then(|d| d.get("code"))
            .and_then(serde_json::Value::as_str),
        &Some("mechanic_pattern_too_broad"),
        "buffered partial trial scan returns mechanic_pattern_too_broad",
    )?;
    ensure(
        buf_manager
            .result_stores()
            .current()
            .check("read buffered result store slot")?
            .is_none(),
        "buffered partial trial scan must never materialize into ResultStore",
    )?;
    Ok(())
}

#[test]
fn mechanic_add_commit_entry_refuses_when_project_disabled_mid_scan() -> TestResult {
    let parent = tempfile::tempdir().check("parent dir")?;
    crate::project::activate_for_test(parent.path())?;
    let child = parent.path().join("child");
    std::fs::create_dir_all(&child).check("create child dir")?;
    crate::project::activate_for_test(&child)?;
    let child_root = crate::project::find_project_root(&child).check("child root")?;
    let limits = crate::tools::limits_for_root(child_root.as_path()).check("limits")?;
    let entry = crate::project::MechanicEntry {
        id: "py_eval".to_owned(),
        language: "python".to_owned(),
        query: EVAL_QUERY.to_owned(),
        note: "dynamic eval".to_owned(),
    };
    let compiled = super::catalog::compile(&entry, &limits).check("compile")?;
    let manager = sync::SourceSyncManager::default();

    manager
        .disable_project(&child_root)
        .check("disable child project mid-scan")?;
    let error = super::register::commit_entry(&manager, child_root.as_path(), entry, compiled)
        .check_err("commit_entry after disable must fail")?;

    ensure_eq(
        &error
            .data
            .as_ref()
            .and_then(|d| d.get("code"))
            .and_then(serde_json::Value::as_str),
        &Some("project_not_enabled"),
        "disabled project commit returns project_not_enabled",
    )?;
    ensure(
        manager
            .session_mechanics()
            .entries(child_root.as_path())
            .check("read session mechanics")?
            .is_empty(),
        "disabled project must not have the mechanic registered",
    )?;
    Ok(())
}

#[test]
fn mechanic_remove_syncs_new_disk_files_into_the_index() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    std::fs::write(project.path().join("added.py"), "x = 1\n").check("write added.py")?;
    call(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    )
    .check("remove")?;
    let count: i64 =
        crate::store::open_existing(&project.path().join(".planning/slugaudit/project.db"))
            .check("open project.db")?
            .query_row(
                "SELECT count(*) FROM files WHERE path = 'added.py'",
                [],
                |r| r.get(0),
            )
            .check("query")?;
    ensure_eq(&count, &1, "mechanic remove syncs added.py into project.db")?;
    Ok(())
}
