//! Custom-mechanic selection, per-project isolation, trial-scan completeness
//! refusal, and `QueryCursor` match-limit / time-budget partial reporting.

use super::tests::{
    EVAL_QUERY, action, add, call, leads_request, path_of, python_project, refusal, report_of,
    run_leads, sources,
};
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::LeadMode;
use crate::tools::page::{Completeness, PageLimit};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

/// Paired `@hit` query whose finished sibling pairs accumulate in `QueryCursor`
/// past `65,536` slots on `dense_let_fn(370)` while finishing cleanly in a few
/// milliseconds (`states.size <= 370`) and returning `2` hits on `dense_let_fn(2)`.
pub(super) const OVERFLOW_LET_QUERY: &str = "(block (let_declaration) @hit (let_declaration) @hit)";

pub(super) fn dense_let_fn(statement_count: usize) -> String {
    let mut source = String::from("pub fn dense() {\n");
    for _ in 0..statement_count {
        source.push_str("    let _ = 0;\n");
    }
    source.push_str("}\n");
    source
}

/// The catalogue and limits for a call whose registered-mechanic budget is
/// already spent: any registered entry it scans stops immediately.
pub(super) fn spent_budget(
    manager: &sync::SourceSyncManager,
    path: &str,
) -> TestResult<(crate::model::ResourceLimits, mechanic_catalog::Catalog)> {
    let root = crate::tools::project_root(path).check("project root")?;
    let mut limits = crate::tools::limits_for_root(&root).check("limits")?;
    limits.max_structure_execution_time = std::time::Duration::ZERO;
    let catalog = mechanic_catalog::load(manager.session_mechanics(), root.as_path())
        .check("load the mechanic catalog")?;
    Ok((limits, catalog))
}

#[test]
fn a_registered_scan_past_its_budget_is_a_partial_result_naming_how_far_it_got() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    std::fs::write(project.path().join("extra.py"), "eval(\"4\")\n").check("write a file")?;
    let (limits, catalog) = spent_budget(&manager, &path)?;
    let request = leads_request(&path, None, LeadMode::Full);

    let response = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .check("a spent budget is a partial result, never a silent complete count")?;

    ensure_eq(
        &response.page.completeness,
        &Some(Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: None,
            files_scanned: 0,
            files_total: 3,
        }),
        "partial completeness reports TimeBudget and 0/3 files scanned",
    )?;
    ensure(
        !response.page.complete,
        "partial single-page scan must report page.complete == false",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::TimeBudget,
        "response.page.limited_by == PageLimit::TimeBudget",
    )?;
    Ok(())
}

#[test]
fn a_leads_call_naming_a_builtin_scans_no_registered_entry() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let (limits, catalog) = spent_budget(&manager, &path)?;
    let request = leads_request(&path, Some("comment_markers"), LeadMode::Full);

    let response = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .check("a built-in page never runs a registered scan, so the spent budget is not hit")?;

    ensure(
        response
            .mechanic_counts
            .iter()
            .all(|count| count.source == MechanicSource::Builtin),
        format!(
            "an unscanned entry is not listed with a made-up count: {:?}",
            sources(&response)
        ),
    )?;
    Ok(())
}

#[test]
fn a_leads_call_naming_one_registered_entry_counts_only_that_one() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let exec_query = EVAL_QUERY.replace("eval", "exec");
    call(&manager, add(&path, "py_exec", "python", &exec_query)).check("add")?;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_exec"), LeadMode::Full),
    )?;

    let registered: Vec<(String, MechanicSource)> = sources(&response)
        .into_iter()
        .filter(|(_, source)| *source != MechanicSource::Builtin)
        .collect();
    ensure_eq(
        &registered,
        &[("py_exec".to_owned(), MechanicSource::Session)],
        "only the requested session mechanic is counted",
    )?;
    Ok(())
}

#[test]
fn removing_and_re_adding_an_id_with_another_query_uses_the_new_query() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    call(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    )
    .check("remove")?;
    let exec_query = EVAL_QUERY.replace("eval", "exec");

    call(&manager, add(&path, "py_eval", "python", &exec_query)).check("re-add")?;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    )?;
    ensure_eq(
        &response.page.total,
        &Some(0),
        "re-added mechanic scans with the new query",
    )?;
    Ok(())
}

#[test]
fn a_different_pattern_glob_on_the_same_revision_returns_its_own_count() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let mut narrowed = leads_request(&path, Some("py_eval"), LeadMode::Full);
    narrowed.pattern = Some("lib".to_owned());

    let response = run_leads(&manager, narrowed)?;

    ensure_eq(
        &response.page.total,
        &Some(1),
        "narrowed pattern counts only lib/util.py",
    )?;
    Ok(())
}

#[test]
fn mechanic_add_refuses_partial_trial_scan_on_query_cursor_match_limit_and_time_budget()
-> TestResult {
    let project =
        crate::tools::test_support::activated_project("dense.rs", dense_let_fn(370).as_bytes())?;
    let path = path_of(&project)?;
    let root = crate::tools::project_root(&path).check("project root")?;
    let manager = sync::SourceSyncManager::default();

    let overflow_err = refusal(
        &manager,
        add(&path, "let_quads", "rust", OVERFLOW_LET_QUERY),
    )?;
    ensure_eq(
        &overflow_err.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "overflowing trial scan is invalid_params",
    )?;
    ensure(
        overflow_err.message.contains("mechanic not added")
            && overflow_err.message.contains("QueryCursor match limit"),
        format!(
            "unexpected overflow error message: {}",
            overflow_err.message
        ),
    )?;
    let payload = overflow_err.data.check("structured error payload")?;
    ensure_eq(
        &payload.get("code").and_then(serde_json::Value::as_str),
        &Some("mechanic_pattern_too_broad"),
        "error code is mechanic_pattern_too_broad",
    )?;
    ensure(
        payload
            .get("remedy")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|remedy| remedy.contains("Narrow the Tree-sitter query")),
        "remedy advises narrowing the Tree-sitter pattern",
    )?;
    ensure_eq(
        &manager
            .session_mechanics()
            .entries(root.as_path())
            .check("read entries")?
            .len(),
        &0,
        "refused mechanic must not be registered",
    )?;

    let mut zero_budget = crate::tools::limits_for_root(&root).check("limits")?;
    zero_budget.max_structure_execution_time = std::time::Duration::ZERO;
    let timeout_err = super::register::add(
        &add(
            &path,
            "fn_names",
            "rust",
            "(function_item name: (identifier) @name)",
        ),
        &path,
        root.as_path(),
        &zero_budget,
        &NoopProgressSink,
        &manager,
    )
    .err()
    .check("timed-out trial scan must be refused")?;
    let timeout_payload = timeout_err.data.check("structured timeout payload")?;
    ensure_eq(
        &timeout_payload
            .get("code")
            .and_then(serde_json::Value::as_str),
        &Some("mechanic_pattern_too_broad"),
        "timed-out trial scan also carries mechanic_pattern_too_broad",
    )?;
    ensure_eq(
        &manager
            .session_mechanics()
            .entries(root.as_path())
            .check("read entries after timeout refusal")?
            .len(),
        &0,
        "timed-out mechanic must not be registered",
    )?;
    Ok(())
}

#[test]
fn leads_marks_completeness_partial_when_custom_mechanic_exceeds_query_cursor_match_limit()
-> TestResult {
    let project =
        crate::tools::test_support::activated_project("a_small.rs", dense_let_fn(2).as_bytes())?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let added = call(
        &manager,
        add(&path, "let_quads", "rust", OVERFLOW_LET_QUERY),
    )
    .check("2 statements yield 2 hits and register cleanly")?;
    let initial = added.first_count.check("first_count")?;
    ensure_eq(&initial.page.total, &Some(2), "initial match count is 2")?;
    ensure_eq(
        &initial.page.completeness,
        &Some(Completeness::Complete),
        "initial trial scan is Complete",
    )?;

    std::fs::write(project.path().join("b_dense.rs"), dense_let_fn(370))
        .check("write dense second file")?;
    let after = run_leads(
        &manager,
        leads_request(&path, Some("let_quads"), LeadMode::Full),
    )?;

    ensure_eq(
        &after.page.completeness,
        &Some(Completeness::Partial {
            limited_by: PageLimit::QueryMatchLimit,
            resume_after_file: None,
            files_scanned: 1,
            files_total: 2,
        }),
        "leads must mark completeness Partial with QueryMatchLimit when QueryCursor overflows",
    )?;
    Ok(())
}

#[test]
fn custom_mechanics_are_isolated_per_project_across_add_list_remove_leads_and_report() -> TestResult
{
    let project_a = python_project()?;
    let project_b = python_project()?;
    let path_a = path_of(&project_a)?;
    let path_b = path_of(&project_b)?;
    let manager = sync::SourceSyncManager::default();

    call(&manager, add(&path_a, "py_eval", "python", EVAL_QUERY)).check("add on project A")?;

    let list_b =
        call(&manager, action(&path_b, MechanicAction::List, None)).check("list on project B")?;
    ensure(
        list_b
            .mechanics
            .iter()
            .all(|entry| entry.source == MechanicSource::Builtin),
        "project A's custom mechanic must never appear in project B's mechanic list",
    )?;

    let leads_b = run_leads(&manager, leads_request(&path_b, None, LeadMode::Full))?;
    ensure(
        leads_b
            .mechanic_counts
            .iter()
            .all(|count| count.source == MechanicSource::Builtin),
        "project A's custom mechanic must never appear in project B's leads",
    )?;

    let report_a = report_of(&manager, &path_a)?;
    let report_b = report_of(&manager, &path_b)?;
    ensure(
        report_a
            .mechanics
            .entries
            .iter()
            .any(|entry| entry.id == "py_eval"),
        "project A's report lists py_eval",
    )?;
    ensure(
        report_b
            .mechanics
            .entries
            .iter()
            .all(|entry| entry.id != "py_eval"),
        "project B's report does not list project A's py_eval",
    )?;
    ensure(
        report_a.mechanics.note.is_some() && report_b.mechanics.note.is_some(),
        "each project receives its own first-report mechanics note",
    )?;

    let remove_b_err = refusal(
        &manager,
        action(&path_b, MechanicAction::Remove, Some("py_eval")),
    )?;
    ensure(
        remove_b_err
            .message
            .contains("no registered mechanic has id `py_eval`"),
        "removing project A's mechanic from project B must fail with unknown_mechanic",
    )?;
    let list_a =
        call(&manager, action(&path_a, MechanicAction::List, None)).check("list on project A")?;
    ensure(
        list_a.mechanics.iter().any(|entry| entry.id == "py_eval"),
        "failed remove on project B leaves project A's mechanic intact",
    )?;
    Ok(())
}
