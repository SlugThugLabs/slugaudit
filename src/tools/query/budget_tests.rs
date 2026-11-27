//! Paging by byte budget, inactive project and step or wall-clock aborts of the query tool.
use super::tests::{activated_project, ask, ask_with_limits};
use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::time::Duration;

const COUNT_TO_900: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL \
    SELECT x + 1 FROM cnt WHERE x < 900) SELECT x FROM cnt";

#[test]
fn more_than_500_rows_come_back_in_one_page_when_the_byte_budget_allows() -> TestResult {
    let project = activated_project(&[])?;

    let response = ask(&project, COUNT_TO_900).check("recursive CTE query succeeds")?;

    ensure_eq(&response.rows.len(), &900, "response.rows.len() == 900")?;
    ensure(response.page.complete, "response.page.complete")?;
    ensure_eq(&response.page.cursor, &None, "response.page.cursor == None")?;
    Ok(())
}

#[test]
fn a_page_is_cut_by_the_byte_budget_and_carries_a_cursor() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits {
        max_query_response_bytes: 200,
        ..ResourceLimits::default()
    };

    let first = ask_with_limits(&project, COUNT_TO_900, &limits).check("first page")?;

    ensure(!first.page.complete, "!first.page.complete")?;
    ensure(
        first.rows.len() > 1 && first.rows.len() < 900,
        "first.rows.len() > 1 && first.rows.len() < 900",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;
    ensure_eq(
        &first
            .rows
            .first()
            .check("first.rows[0]")?
            .get("x")
            .check("first.rows[0][\"x\"]")?,
        &1,
        "first.rows[0][\"x\"] == 1",
    )?;
    Ok(())
}

#[test]
fn an_agent_limit_and_offset_still_work_inside_the_subquery() -> TestResult {
    let project = activated_project(&[])?;

    let response = ask(&project, &format!("{COUNT_TO_900} LIMIT 3 OFFSET 10;"))
        .check("the agent's own LIMIT/OFFSET is honored")?;

    let xs: Vec<_> = response.rows.iter().map(|r| r["x"].clone()).collect();
    ensure_eq(&xs, &[11, 12, 13], "xs == [11, 12, 13]")?;
    ensure(response.page.complete, "response.page.complete")?;
    Ok(())
}

#[test]
fn query_on_an_inactive_project_is_a_typed_error_not_a_panic() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    let Err(error) = ask(&project, "SELECT 1") else {
        return Err("an inactive project must fail".to_owned()).check("unexpected outcome");
    };
    // The disabled state is one instruction, not one opaque failure per
    // tool: the prose stays, and the machine-readable half says *which*
    // state this is and which call leaves it.
    ensure(
        error.message.contains("not enabled"),
        format!("the existing wording stays: {}", error.message),
    )?;
    let data = error.data.check("the disabled state carries a payload")?;
    ensure_eq(
        &data.get("code").check("data[\"code\"]")?,
        &"project_not_enabled",
        "data[\"code\"] == \"project_not_enabled\"",
    )?;
    let remedy = data
        .get("remedy")
        .check("data[\"remedy\"]")?
        .as_str()
        .check("remedy is a string")?;
    ensure(remedy.contains("project_control"), remedy)?;
    ensure(remedy.contains("\"action\":\"on\""), remedy)?;
    Ok(())
}

#[test]
fn an_oversized_text_or_blob_value_is_rejected_before_being_expanded() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits {
        max_query_value_bytes: 1024,
        ..ResourceLimits::default()
    };
    // zeroblob/printf avoid embedding a huge literal in the test source:
    // SQLite materializes the oversized value, and the per-value cap must
    // reject it before it is cloned or hex-expanded into JSON.
    for sql in [
        "SELECT zeroblob(4096) AS huge",
        "SELECT printf('%.*c', 4096, 'x') AS huge",
    ] {
        let error =
            ask_with_limits(&project, sql, &limits).check_err("oversized value is rejected")?;
        ensure(
            error.message.contains("per-value cap"),
            format!("unexpected message for {sql}: {}", error.message),
        )?;
    }
    Ok(())
}

#[test]
fn a_pathological_query_is_aborted_by_the_step_budget() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits {
        max_query_vm_steps: 10,
        ..ResourceLimits::default()
    };
    let result = ask_with_limits(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5000) SELECT x FROM cnt",
        &limits,
    );
    let error = result.check_err("runaway query is aborted, not run to completion")?;
    ensure(
        error.message.contains("step budget"),
        format!("unexpected message: {}", error.message),
    )?;
    Ok(())
}

#[test]
fn a_pathological_query_is_aborted_by_the_wall_clock_budget() -> TestResult {
    let project = activated_project(&[])?;
    // Effectively zero: by the time the progress handler first fires (every
    // 1000 VM steps), any positive elapsed time trips this deadline, while
    // the step budget stays at its generous default so it is not what
    // catches the query.
    let limits = ResourceLimits {
        max_query_wall_clock: Duration::from_nanos(1),
        ..ResourceLimits::default()
    };
    let manager = crate::sync::SourceSyncManager::default();
    let result = query_with_limits(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5000) SELECT x FROM cnt".to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        &limits,
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .map(|Json(response)| response);
    let error = result.check_err("runaway query is aborted, not run to completion")?;
    ensure(
        error.message.contains("time budget"),
        format!("unexpected message: {}", error.message),
    )?;
    if let Some(store) = manager.result_stores().current().check("the store slot")? {
        let stats = store.stats().check("read the store counters")?;
        ensure_eq(
            &(
                stats.active.results,
                stats.idle.results,
                stats.delivered.results,
            ),
            &(0, 0, 0),
            "no partial result is left behind",
        )?;
    }
    Ok(())
}

#[test]
fn results_exceeding_response_bytes_are_cut_into_pages() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits {
        max_query_response_bytes: 50,
        ..ResourceLimits::default()
    };
    let response = ask_with_limits(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 50) SELECT printf('long_string_value_%d', x) AS s FROM cnt",
        &limits,
    )
    .check("query succeeds")?;
    ensure(!response.page.complete, "!response.page.complete")?;
    ensure(!response.rows.is_empty(), "!response.rows.is_empty()")?;
    ensure(response.rows.len() < 50, "response.rows.len() < 50")?;
    Ok(())
}
