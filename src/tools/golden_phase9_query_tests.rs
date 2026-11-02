//! Phase 9 golden responses for `query`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, mask_paging, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{LegacyPaging, QueryRequest, query};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use slugaudit_test_check::{Check, TestResult};

fn run_query(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    sql: &str,
    page_bytes: Option<usize>,
) -> TestResult<Value> {
    call(manager, Some(path_of(project)?), sql, None, page_bytes)
}

fn run_cursor(
    manager: &SourceSyncManager,
    cursor: &str,
    page_bytes: Option<usize>,
) -> TestResult<Value> {
    call(manager, None, "", Some(cursor.to_owned()), page_bytes)
}

fn call(
    manager: &SourceSyncManager,
    path: Option<String>,
    sql: &str,
    cursor: Option<String>,
    page_bytes: Option<usize>,
) -> TestResult<Value> {
    let response = query(
        &Parameters(QueryRequest {
            path,
            sql: sql.to_owned(),
            cursor,
            legacy_paging: LegacyPaging::default(),
            page_bytes,
        }),
        &NoopProgressSink,
        manager,
    )
    .check("query succeeds")?
    .0;
    serde_json::to_value(response).check("response serializes")
}

fn query_case(name: &str, sql: &str, page_bytes: Option<usize>) -> TestResult {
    let project = rust_project()?;
    let manager = SourceSyncManager::default();
    let response = run_query(&project, &manager, sql, page_bytes)?;
    check_case(
        name,
        &project,
        &json!({ "sql": sql, "page_bytes": page_bytes }),
        &response,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_plain_rows() -> TestResult {
    query_case(
        "query_plain_rows",
        "SELECT path, language FROM files ORDER BY path",
        None,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_clipped_first_row() -> TestResult {
    query_case(
        "query_clipped_cells",
        "SELECT replace(hex(zeroblob(1500)), '00', 'ab') AS t, zeroblob(1500) AS b, 7 AS n",
        Some(600),
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_positive_infinity() -> TestResult {
    query_case(
        "query_non_finite_inf",
        "SELECT 1e308*10 AS pos, -1e308*10 AS neg, 1.5 AS ok",
        None,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_nan_expressions() -> TestResult {
    query_case(
        "query_nan_expressions",
        "SELECT 1e308*10*0 AS mul, 1e308*10 - 1e308*10 AS sub, golden_nan() AS udf",
        None,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_text_with_nul() -> TestResult {
    query_case("query_text_with_nul", "SELECT 'a'||char(0)||'b' AS s", None)?;
    Ok(())
}

#[test]
fn golden_phase9_query_blob() -> TestResult {
    query_case(
        "query_blob",
        "SELECT x'00ff' AS b, zeroblob(0) AS empty",
        None,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_numeric_looking_text() -> TestResult {
    query_case(
        "query_numeric_text",
        "SELECT '007' AS a, '1e3' AS b, '-0' AS c, ' 5' AS d",
        None,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_null_and_integers() -> TestResult {
    query_case(
        "query_null",
        "SELECT NULL AS n, 1 AS one, 0.5 AS half",
        None,
    )?;
    query_case(
        "query_i64_extremes",
        "SELECT 9223372036854775807 AS max_i, -9223372036854775808 AS min_i, 9007199254740993 AS past_2_53, 1e15 AS big_real",
        None,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_query_multi_page() -> TestResult {
    let project = rust_project()?;
    let manager = SourceSyncManager::default();
    let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<60) SELECT i, printf('%050d', i) AS pad FROM n";
    let one = run_query(&project, &manager, sql, Some(1000))?;
    check_case(
        "query_multi_page1",
        &project,
        &json!({ "sql": sql, "page_bytes": 1000 }),
        &one,
    )?;
    let cursor = one["page"]["cursor"]
        .as_str()
        .check("a second page exists")?
        .to_owned();
    let two = run_cursor(&manager, &cursor, Some(1000))?;
    check_case(
        "query_multi_page2",
        &project,
        &json!({ "sql": sql, "page_bytes": 1000, "page": 2 }),
        &two,
    )?;
    Ok(())
}

#[test]
fn golden_phase9_mask_paging_keeps_result_columns_aliased_like_paging_keys() -> TestResult {
    let project = rust_project()?;
    let manager = SourceSyncManager::default();
    let mut response = run_query(
        &project,
        &manager,
        "SELECT 'c' AS cursor, 7 AS offset, 8 AS next_offset",
        None,
    )?;

    mask_paging(&mut response);

    assert_eq!(
        response["rows"],
        json!([{ "cursor": "c", "offset": 7, "next_offset": 8 }])
    );
    assert!(response["page"].get("cursor").is_none(), "{response}");
    assert!(response["page"].get("next_offset").is_none(), "{response}");
    Ok(())
}
