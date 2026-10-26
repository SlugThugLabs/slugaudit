//! Which registered entries a `leads` call scans, and how a spent scan
//! budget fails: a call naming a built-in runs no registered scan, a call
//! naming one entry counts only that one, and a spent budget is an
//! operational error naming the entry and carrying `mechanic_budget_exhausted`.
use super::tests::{
    EVAL_QUERY, add, call, leads_request, path_of, python_project, run_leads, sources,
};
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::LeadMode;

/// The catalogue and limits for a call whose registered-mechanic budget is
/// already spent: any registered entry it scans fails it.
fn spent_budget(
    manager: &sync::SourceSyncManager,
    path: &str,
) -> (crate::model::ResourceLimits, mechanic_catalog::Catalog) {
    let mut limits = crate::tools::resolve_limits(path).expect("limits");
    limits.max_structure_execution_time = std::time::Duration::ZERO;
    let root = crate::tools::project_root(path).expect("root");
    let catalog = mechanic_catalog::load(&root, manager.session_mechanics(), &limits);
    (limits, catalog)
}

#[test]
fn a_registered_scan_past_its_budget_fails_the_call_naming_the_entry() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let (limits, catalog) = spent_budget(&manager, &path);
    let request = leads_request(&path, None, LeadMode::Full);

    let error = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .expect_err("a spent budget is an error, never a partial count");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "a spent budget is operational, not a contract error: {error:?}"
    );
    assert!(
        error.message.contains("`py_eval`")
            && error.message.contains("max_structure_execution_time"),
        "{}",
        error.message
    );
    let data = error.data.expect("the budget carries a code");
    assert_eq!(data["code"], "mechanic_budget_exhausted");
    assert!(
        data["remedy"]
            .as_str()
            .is_some_and(|remedy| remedy.contains("mechanic action remove")),
        "{data}"
    );
}

#[test]
fn a_leads_call_naming_a_builtin_scans_no_registered_entry() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let (limits, catalog) = spent_budget(&manager, &path);
    let request = leads_request(&path, Some("unwrap_calls"), LeadMode::Full);

    let response = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .expect("a built-in page never runs a registered scan, so the spent budget is not hit");

    assert!(
        response
            .mechanic_counts
            .iter()
            .all(|count| count.source == MechanicSource::Builtin),
        "an unscanned entry is not listed with a made-up count: {:?}",
        sources(&response)
    );
}

#[test]
fn a_leads_call_naming_one_registered_entry_counts_only_that_one() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let exec_query = EVAL_QUERY.replace("eval", "exec");
    call(&manager, add(&path, "py_exec", "python", &exec_query)).expect("add");

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_exec"), LeadMode::Full),
    );

    let registered: Vec<(String, MechanicSource)> = sources(&response)
        .into_iter()
        .filter(|(_, source)| *source != MechanicSource::Builtin)
        .collect();
    assert_eq!(
        registered,
        [("py_exec".to_owned(), MechanicSource::Session)]
    );
}
