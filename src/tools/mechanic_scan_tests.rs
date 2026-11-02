//! Which registered entries a `leads` call scans, and how a spent scan
//! budget fails: a call naming a built-in runs no registered scan, a call
//! naming one entry counts only that one, and a spent budget is a
//! partial result (or, when rows cannot be reported, an operational error
//! naming the entry and carrying `mechanic_budget_exhausted`).
use super::tests::{
    EVAL_QUERY, add, call, leads_request, path_of, python_project, run_leads, sources,
};
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::LeadMode;
use crate::tools::page::{Completeness, PageLimit};
use slugaudit_test_check::{Check, TestResult};

/// The catalogue and limits for a call whose registered-mechanic budget is
/// already spent: any registered entry it scans fails it.
fn spent_budget(
    manager: &sync::SourceSyncManager,
    path: &str,
) -> TestResult<(crate::model::ResourceLimits, mechanic_catalog::Catalog)> {
    let mut limits = crate::tools::resolve_limits(path).check("limits")?;
    limits.max_structure_execution_time = std::time::Duration::ZERO;
    let catalog = mechanic_catalog::load(manager.session_mechanics(), &limits)
        .check("load the mechanic catalog")?;
    Ok((limits, catalog))
}

#[test]
fn a_registered_scan_past_its_budget_is_a_partial_result_naming_how_far_it_got() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    // A new revision, so the scan is not served from the entry `add` cached.
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

    assert_eq!(
        response.page.completeness,
        Some(Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: None,
            files_scanned: 0,
            files_total: 3,
        })
    );
    assert!(
        response.page.complete,
        "no rows follow: the page is the whole result"
    );
    Ok(())
}

#[test]
fn a_leads_call_naming_a_builtin_scans_no_registered_entry() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let (limits, catalog) = spent_budget(&manager, &path)?;
    let request = leads_request(&path, Some("unwrap_calls"), LeadMode::Full);

    let response = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .check("a built-in page never runs a registered scan, so the spent budget is not hit")?;

    assert!(
        response
            .mechanic_counts
            .iter()
            .all(|count| count.source == MechanicSource::Builtin),
        "an unscanned entry is not listed with a made-up count: {:?}",
        sources(&response)
    );
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
    assert_eq!(
        registered,
        [("py_exec".to_owned(), MechanicSource::Session)]
    );
    Ok(())
}

fn cache_stats(manager: &sync::SourceSyncManager) -> TestResult<crate::project::CacheStats> {
    manager
        .mechanic_scan_cache()
        .stats()
        .check("read the cache counters")
}

fn health_cache(manager: &sync::SourceSyncManager) -> TestResult<serde_json::Value> {
    let request = crate::tools::HealthRequest {
        path: None,
        verbose: true,
    };
    let response = crate::tools::health(
        &rmcp::handler::server::wrapper::Parameters(request),
        &NoopProgressSink,
        manager,
    )
    .check("health succeeds")?
    .0;
    Ok(
        serde_json::to_value(&response).check("health serialises")?["detail"]["mechanic_cache"]
            .clone(),
    )
}

#[test]
fn a_second_leads_call_on_an_unchanged_revision_runs_no_scan() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let scans_after_add = cache_stats(&manager)?.misses;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    )?;

    let stats = cache_stats(&manager)?;
    assert_eq!(stats.misses, scans_after_add, "no further scan ran");
    assert_eq!(stats.hits, 1, "the call was answered from the cache");
    assert_eq!(response.page.total, Some(4));
    let (limits, catalog) = spent_budget(&manager, &path)?;
    let request = leads_request(&path, Some("py_eval"), LeadMode::Full);
    let served = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .check("a cached result needs no time budget, a scan would exhaust it")?;
    assert_eq!(served.page.total, Some(4));
    Ok(())
}

#[test]
fn a_changed_file_invalidates_the_cache_and_the_next_call_returns_the_new_count() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let misses = cache_stats(&manager)?.misses;
    let extra = project.path().join("extra.py");
    std::fs::write(&extra, "eval(\"4\")\neval(\"5\")\n").check("write a new python file")?;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    )?;

    assert_eq!(
        response.page.total,
        Some(6),
        "the new revision's count, not the cached 4"
    );
    let stats = cache_stats(&manager)?;
    assert_eq!(
        stats.misses,
        misses + 1,
        "the changed revision scanned again"
    );
    assert_eq!(stats.entries, 1, "the old revision's entry was dropped");
    Ok(())
}

#[test]
fn removing_and_re_adding_an_id_with_another_query_does_not_hit_the_old_entry() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    call(
        &manager,
        super::tests::action(&path, crate::tools::MechanicAction::Remove, Some("py_eval")),
    )
    .check("remove")?;
    assert_eq!(cache_stats(&manager)?.entries, 0, "remove dropped the id");
    let exec_query = EVAL_QUERY.replace("eval", "exec");

    call(&manager, add(&path, "py_eval", "python", &exec_query)).check("re-add")?;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    )?;
    assert_eq!(
        response.page.total,
        Some(0),
        "the new query's count, not the old 4"
    );
    Ok(())
}

#[test]
fn inserting_past_the_budget_evicts_the_oldest_entry_and_health_reports_it() -> TestResult {
    let sizing = sync::SourceSyncManager::default();
    let project = python_project()?;
    let path = path_of(&project)?;
    call(&sizing, add(&path, "first", "python", EVAL_QUERY)).check("add")?;
    let one_entry = cache_stats(&sizing)?.bytes;
    // One manager per project at a time: its result store owns the project's
    // results directory.
    drop(sizing);
    let manager = sync::SourceSyncManager::with_mechanic_cache_budget(one_entry + one_entry / 2);

    call(&manager, add(&path, "first", "python", EVAL_QUERY)).check("add")?;
    call(&manager, add(&path, "second", "python", EVAL_QUERY)).check("add")?;

    let stats = cache_stats(&manager)?;
    assert_eq!((stats.entries, stats.evictions), (1, 1));
    let health = health_cache(&manager)?;
    assert_eq!(health["evictions"], 1, "{health}");
    assert_eq!(health["entries"], 1, "{health}");
    assert_eq!(health["budget"], one_entry + one_entry / 2, "{health}");
    Ok(())
}

#[test]
fn an_entry_larger_than_the_whole_budget_is_skipped_counted_and_still_answered() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::with_mechanic_cache_budget(64);

    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    )?;
    assert_eq!(
        response.page.total,
        Some(4),
        "an uncached entry still answers in full"
    );
    let health = health_cache(&manager)?;
    assert_eq!(health["entries"], 0, "{health}");
    assert_eq!(health["skipped_too_large"], 2, "{health}");
    Ok(())
}

/// Every page of `py_eval` in `mode`, cursors blanked (they name the
/// manager's own stored result).
fn py_eval_pages(
    manager: &sync::SourceSyncManager,
    path: &str,
    mode: LeadMode,
) -> TestResult<Vec<serde_json::Value>> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let mut request = leads_request(path, Some("py_eval"), mode);
        request.limit = Some(3);
        request.cursor.clone_from(&cursor);
        let page = run_leads(manager, request)?;
        cursor.clone_from(&page.page.cursor);
        let mut value = serde_json::to_value(&page).check("serialises")?;
        value["page"]["cursor"] = serde_json::Value::Null;
        pages.push(value);
        if cursor.is_none() {
            return Ok(pages);
        }
    }
}

#[test]
fn paging_from_the_cache_equals_paging_from_a_fresh_scan() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    // One manager per project at a time (its result store owns the project's
    // results directory), so the two are used one after the other.
    let cached = sync::SourceSyncManager::default();
    call(&cached, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let from_cache: Vec<_> = [LeadMode::Full, LeadMode::Compact]
        .into_iter()
        .map(|mode| py_eval_pages(&cached, &path, mode))
        .collect::<TestResult<_>>()?;
    assert!(
        cache_stats(&cached)?.hits >= 2,
        "the cached manager used its cache"
    );
    drop(cached);
    let fresh = sync::SourceSyncManager::with_mechanic_cache_budget(0);
    call(&fresh, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let from_scan: Vec<_> = [LeadMode::Full, LeadMode::Compact]
        .into_iter()
        .map(|mode| py_eval_pages(&fresh, &path, mode))
        .collect::<TestResult<_>>()?;

    assert_eq!(from_cache, from_scan);
    assert!(from_cache.iter().all(|pages| pages.len() >= 2), "it paged");
    assert_eq!(cache_stats(&fresh)?.entries, 0, "the other held nothing");
    Ok(())
}

#[test]
fn a_scan_cut_by_the_time_budget_is_not_cached() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    std::fs::write(project.path().join("extra.py"), "eval(\"4\")\n").check("write a file")?;
    let (limits, catalog) = spent_budget(&manager, &path)?;
    let request = leads_request(&path, Some("py_eval"), LeadMode::Full);

    let response = crate::tools::leads::leads_with_catalog(
        &request,
        &limits,
        &catalog,
        &NoopProgressSink,
        &manager,
    )
    .check("the new revision needs a scan and the budget is spent: a partial result")?;

    assert!(
        matches!(
            response.page.completeness,
            Some(Completeness::Partial { .. })
        ),
        "{:?}",
        response.page.completeness
    );
    assert_eq!(
        cache_stats(&manager)?.entries,
        0,
        "nothing partial was kept"
    );
    Ok(())
}

fn count_of(response: &crate::tools::LeadsResponse, id: &str) -> TestResult<i64> {
    let found = response
        .mechanic_counts
        .iter()
        .find(|count| count.mechanic == id)
        .check(&format!("no count for {id}"))?;
    Ok(found.count)
}

#[test]
fn a_different_pattern_glob_on_the_same_revision_is_a_miss_with_its_own_count() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let misses = cache_stats(&manager)?.misses;
    let mut narrowed = leads_request(&path, Some("py_eval"), LeadMode::Full);
    narrowed.pattern = Some("lib".to_owned());

    let response = run_leads(&manager, narrowed)?;

    assert_eq!(
        response.page.total,
        Some(1),
        "only lib/util.py is under the pattern"
    );
    assert_eq!(
        cache_stats(&manager)?.misses,
        misses + 1,
        "it scanned again"
    );
    Ok(())
}

#[test]
fn a_second_project_on_one_manager_does_not_hit_the_first_projects_entry() -> TestResult {
    let first = python_project()?;
    let second = python_project()?;
    std::fs::write(second.path().join("extra.py"), "eval(\"9\")\n").check("write extra")?;
    let (first_path, second_path) = (path_of(&first)?, path_of(&second)?);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&first_path, "py_eval", "python", EVAL_QUERY)).check("add first")?;
    let misses = cache_stats(&manager)?.misses;

    let response = run_leads(
        &manager,
        leads_request(&second_path, Some("py_eval"), LeadMode::Full),
    )?;

    assert_eq!(
        response.page.total,
        Some(5),
        "the second project's own count"
    );
    assert_eq!(
        cache_stats(&manager)?.misses,
        misses + 1,
        "the second project scanned its own files instead of hitting the first's entry"
    );
    Ok(())
}

#[test]
fn a_pass_over_many_entries_retains_at_most_the_budget_in_total() {
    use crate::tools::mechanic_scan_cached::Pass;
    let row = crate::project::CachedRow {
        file: "a.py".to_owned(),
        line: 1,
        text: "x".to_owned(),
    };
    let size = row.estimated_bytes();
    let mut pass = Pass::new(size * 5, 4);

    for _ in 0..3 {
        for entry in 0..4 {
            pass.offer(entry, row.clone());
            assert!(
                pass.retained_bytes() <= size * 5,
                "retained {} past the shared budget",
                pass.retained_bytes()
            );
        }
    }

    assert!(pass.retained_bytes() <= size * 5);
}

#[test]
fn many_uncached_entries_in_one_call_over_a_tiny_budget_are_all_counted_exactly() -> TestResult {
    let sizing = sync::SourceSyncManager::default();
    let project = python_project()?;
    let path = path_of(&project)?;
    call(&sizing, add(&path, "sizing", "python", EVAL_QUERY)).check("add")?;
    let one_entry = cache_stats(&sizing)?.bytes;
    let budget = one_entry + one_entry / 2;
    drop(sizing);
    let manager = sync::SourceSyncManager::with_mechanic_cache_budget(budget);
    for id in ["e1", "e2", "e3", "e4", "e5"] {
        call(&manager, add(&path, id, "python", EVAL_QUERY)).check("add")?;
    }
    std::fs::write(
        project.path().join("extra.py"),
        "eval(\"4\")\neval(\"5\")\n",
    )
    .check("new revision")?;
    let skipped_before = cache_stats(&manager)?.skipped_too_large;

    let response = run_leads(&manager, leads_request(&path, None, LeadMode::Full))?;

    for id in ["e1", "e2", "e3", "e4", "e5"] {
        assert_eq!(count_of(&response, id)?, 6, "{id}");
    }
    let stats = cache_stats(&manager)?;
    assert!(stats.bytes <= budget, "{} > {budget}", stats.bytes);
    assert!(
        stats.skipped_too_large > skipped_before,
        "entries past the pass budget were counted, not cached"
    );
    Ok(())
}

#[test]
fn a_selected_index_outside_the_catalogue_fails_the_call_instead_of_being_skipped() -> TestResult {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;
    let tx = connection.unchecked_transaction().check("transaction")?;
    let cache = crate::project::MechanicScanCache::with_budget(1024);
    let scope = crate::tools::mechanic_scan_cached::CacheScope {
        cache: &cache,
        project: "p",
        revision: "r",
    };

    let Err(error) =
        crate::tools::mechanic_scan_cached::tally_cached(&tx, &[], &[0], None, None, &scope)
    else {
        return Err("an out-of-range index must fail the call".to_owned())
            .check("unexpected outcome");
    };

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with(
            "counting mechanics: entry index 0 is out of range for custom of length 0."
        ),
        "{}",
        error.message
    );
    Ok(())
}

#[path = "mechanic_scan_stop_tests.rs"]
mod stop_tests;
