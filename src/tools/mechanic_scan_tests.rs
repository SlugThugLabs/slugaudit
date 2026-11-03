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
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

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

    ensure_eq(
        &response.page.completeness,
        &Some(Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: None,
            files_scanned: 0,
            files_total: 3,
        }),
        "response.page.completeness == Some(Completeness::Partial { limited_by: PageLimit::TimeBudget, resume_after_file: None, files_scanned: 0, files_total: 3, })",
    )?;
    ensure(response.page.complete, "response.page.complete")?;
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
        "registered == [(\"py_exec\".to_owned(), MechanicSource::Session)]",
    )?;
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
    Ok(serde_json::to_value(&response)
        .check("health serialises")?
        .get("detail")
        .check("...alises\")?[\"detail\"][\"mechanic_cache\"]")?
        .get("mechanic_cache")
        .check("...alises\")?[\"detail\"][\"mechanic_cache\"]")?
        .clone())
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
    ensure_eq(
        &stats.misses,
        &scans_after_add,
        "stats.misses == scans_after_add",
    )?;
    ensure_eq(&stats.hits, &1, "stats.hits == 1")?;
    ensure_eq(
        &response.page.total,
        &Some(4),
        "response.page.total == Some(4)",
    )?;
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
    ensure_eq(&served.page.total, &Some(4), "served.page.total == Some(4)")?;
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

    ensure_eq(
        &response.page.total,
        &Some(6),
        "response.page.total == Some(6)",
    )?;
    let stats = cache_stats(&manager)?;
    ensure_eq(&stats.misses, &(misses + 1), "stats.misses == misses + 1")?;
    ensure_eq(&stats.entries, &1, "stats.entries == 1")?;
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
    ensure_eq(
        &cache_stats(&manager)?.entries,
        &0,
        "cache_stats(&manager)?.entries == 0",
    )?;
    let exec_query = EVAL_QUERY.replace("eval", "exec");

    call(&manager, add(&path, "py_eval", "python", &exec_query)).check("re-add")?;

    let response = run_leads(
        &manager,
        leads_request(&path, Some("py_eval"), LeadMode::Full),
    )?;
    ensure_eq(
        &response.page.total,
        &Some(0),
        "response.page.total == Some(0)",
    )?;
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
    ensure_eq(
        &(stats.entries, stats.evictions),
        &(1, 1),
        "(stats.entries, stats.evictions) == (1, 1)",
    )?;
    let health = health_cache(&manager)?;
    ensure_eq(
        health.get("evictions").check("health[\"evictions\"]")?,
        &1,
        "health[\"evictions\"] == 1",
    )?;
    ensure_eq(
        health.get("entries").check("health[\"entries\"]")?,
        &1,
        "health[\"entries\"] == 1",
    )?;
    ensure_eq(
        health.get("budget").check("health[\"budget\"]")?,
        &(one_entry + one_entry / 2),
        "health[\"budget\"] == one_entry + one_entry / 2",
    )?;
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
    ensure_eq(
        &response.page.total,
        &Some(4),
        "response.page.total == Some(4)",
    )?;
    let health = health_cache(&manager)?;
    ensure_eq(
        health.get("entries").check("health[\"entries\"]")?,
        &0,
        "health[\"entries\"] == 0",
    )?;
    ensure_eq(
        health
            .get("skipped_too_large")
            .check("health[\"skipped_too_large\"]")?,
        &2,
        "health[\"skipped_too_large\"] == 2",
    )?;
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
        *value
            .pointer_mut("/page/cursor")
            .check("value[\"page\"][\"cursor\"]")? = serde_json::Value::Null;
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
    ensure(
        cache_stats(&cached)?.hits >= 2,
        "cache_stats(&cached)?.hits >= 2",
    )?;
    drop(cached);
    let fresh = sync::SourceSyncManager::with_mechanic_cache_budget(0);
    call(&fresh, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let from_scan: Vec<_> = [LeadMode::Full, LeadMode::Compact]
        .into_iter()
        .map(|mode| py_eval_pages(&fresh, &path, mode))
        .collect::<TestResult<_>>()?;

    ensure_eq(&from_cache, &from_scan, "from_cache == from_scan")?;
    ensure(
        from_cache.iter().all(|pages| pages.len() >= 2),
        "from_cache.iter().all(|pages| pages.len() >= 2)",
    )?;
    ensure_eq(
        &cache_stats(&fresh)?.entries,
        &0,
        "cache_stats(&fresh)?.entries == 0",
    )?;
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

    ensure(
        matches!(
            response.page.completeness,
            Some(Completeness::Partial { .. })
        ),
        format!("{:?}", response.page.completeness),
    )?;
    ensure_eq(
        &cache_stats(&manager)?.entries,
        &0,
        "cache_stats(&manager)?.entries == 0",
    )?;
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

    ensure_eq(
        &response.page.total,
        &Some(1),
        "response.page.total == Some(1)",
    )?;
    ensure_eq(
        &cache_stats(&manager)?.misses,
        &(misses + 1),
        "cache_stats(&manager)?.misses == misses + 1",
    )?;
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

    ensure_eq(
        &response.page.total,
        &Some(5),
        "response.page.total == Some(5)",
    )?;
    ensure_eq(
        &cache_stats(&manager)?.misses,
        &(misses + 1),
        "cache_stats(&manager)?.misses == misses + 1",
    )?;
    Ok(())
}

#[path = "mechanic_scan_stop_tests.rs"]
mod stop_tests;

#[path = "mechanic_scan_retention_tests.rs"]
mod retention_tests;
