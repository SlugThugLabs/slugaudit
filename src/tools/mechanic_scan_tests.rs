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
    // A new revision, so the scan is not served from the entry `add` cached.
    std::fs::write(project.path().join("extra.py"), "eval(\"4\")\n").expect("write a file");
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

fn cache_stats(manager: &sync::SourceSyncManager) -> crate::project::CacheStats {
    manager.mechanic_scan_cache().stats()
}

fn health_cache(manager: &sync::SourceSyncManager) -> serde_json::Value {
    let request = crate::tools::HealthRequest { path: None };
    let response = crate::tools::health(
        &rmcp::handler::server::wrapper::Parameters(request),
        &NoopProgressSink,
        manager,
    )
    .expect("health succeeds")
    .0;
    serde_json::to_value(&response).expect("health serialises")["mechanic_cache"].clone()
}

#[test]
fn a_second_leads_call_on_an_unchanged_revision_runs_no_scan() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let scans_after_add = cache_stats(&manager).misses;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    );

    let stats = cache_stats(&manager);
    assert_eq!(stats.misses, scans_after_add, "no further scan ran");
    assert_eq!(stats.hits, 1, "the call was answered from the cache");
    assert_eq!(response.total, 4);
    let (limits, catalog) = spent_budget(&manager, &path);
    let request = leads_request(&path, Some("py_eval"), LeadMode::Full);
    let served = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .expect("a cached result needs no time budget, a scan would exhaust it");
    assert_eq!(served.total, 4);
}

#[test]
fn a_changed_file_invalidates_the_cache_and_the_next_call_returns_the_new_count() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let misses = cache_stats(&manager).misses;
    let extra = project.path().join("extra.py");
    std::fs::write(&extra, "eval(\"4\")\neval(\"5\")\n").expect("write a new python file");

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    );

    assert_eq!(
        response.total, 6,
        "the new revision's count, not the cached 4"
    );
    let stats = cache_stats(&manager);
    assert_eq!(
        stats.misses,
        misses + 1,
        "the changed revision scanned again"
    );
    assert_eq!(stats.entries, 1, "the old revision's entry was dropped");
}

#[test]
fn removing_and_re_adding_an_id_with_another_query_does_not_hit_the_old_entry() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    call(
        &manager,
        super::tests::action(&path, crate::tools::MechanicAction::Remove, Some("py_eval")),
    )
    .expect("remove");
    assert_eq!(cache_stats(&manager).entries, 0, "remove dropped the id");
    let exec_query = EVAL_QUERY.replace("eval", "exec");

    call(&manager, add(&path, "py_eval", "python", &exec_query)).expect("re-add");

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    );
    assert_eq!(response.total, 0, "the new query's count, not the old 4");
}

#[test]
fn inserting_past_the_budget_evicts_the_oldest_entry_and_health_reports_it() {
    let sizing = sync::SourceSyncManager::default();
    let project = python_project();
    let path = path_of(&project);
    call(&sizing, add(&path, "first", "python", EVAL_QUERY)).expect("add");
    let one_entry = cache_stats(&sizing).bytes;
    let manager = sync::SourceSyncManager::with_mechanic_cache_budget(one_entry + one_entry / 2);

    call(&manager, add(&path, "first", "python", EVAL_QUERY)).expect("add");
    call(&manager, add(&path, "second", "python", EVAL_QUERY)).expect("add");

    let stats = cache_stats(&manager);
    assert_eq!((stats.entries, stats.evictions), (1, 1));
    let health = health_cache(&manager);
    assert_eq!(health["evictions"], 1, "{health}");
    assert_eq!(health["entries"], 1, "{health}");
    assert_eq!(health["budget"], one_entry + one_entry / 2, "{health}");
}

#[test]
fn an_entry_larger_than_the_whole_budget_is_skipped_counted_and_still_answered() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::with_mechanic_cache_budget(64);

    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    );
    assert_eq!(response.total, 4, "an uncached entry still answers in full");
    let health = health_cache(&manager);
    assert_eq!(health["entries"], 0, "{health}");
    assert_eq!(health["skipped_too_large"], 2, "{health}");
}

#[test]
fn paging_from_the_cache_equals_paging_from_a_fresh_scan() {
    let project = python_project();
    let path = path_of(&project);
    let cached = sync::SourceSyncManager::default();
    let fresh = sync::SourceSyncManager::with_mechanic_cache_budget(0);
    call(&cached, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    call(&fresh, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");

    for mode in [LeadMode::Full, LeadMode::Compact] {
        let mut offset = None;
        loop {
            let page = |manager: &sync::SourceSyncManager| {
                let mut request = leads_request(&path, Some("py_eval"), mode);
                request.limit = Some(3);
                request.offset = offset;
                run_leads(manager, request)
            };
            let from_cache = page(&cached);
            let from_scan = page(&fresh);
            assert_eq!(
                serde_json::to_value(&from_cache).expect("serialises"),
                serde_json::to_value(&from_scan).expect("serialises"),
                "the page at offset {offset:?} in {mode:?}"
            );
            offset = from_cache.next_offset;
            if offset.is_none() {
                break;
            }
        }
    }
    assert!(
        cache_stats(&cached).hits >= 2,
        "the cached manager used its cache"
    );
    assert_eq!(cache_stats(&fresh).entries, 0, "the other held nothing");
}

#[test]
fn a_scan_cut_by_the_time_budget_is_not_cached() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    std::fs::write(project.path().join("extra.py"), "eval(\"4\")\n").expect("write a file");
    let (limits, catalog) = spent_budget(&manager, &path);
    let request = leads_request(&path, Some("py_eval"), LeadMode::Full);

    let error = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .expect_err("the new revision needs a scan and the budget is spent");

    assert_eq!(
        error.data.expect("a code")["code"],
        "mechanic_budget_exhausted"
    );
    assert_eq!(cache_stats(&manager).entries, 0, "nothing partial was kept");
}
