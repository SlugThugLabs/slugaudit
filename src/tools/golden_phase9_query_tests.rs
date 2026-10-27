//! Phase 9 golden responses for `query`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{QueryRequest, query};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};

fn run_query(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    sql: &str,
    offset: usize,
    page_bytes: Option<usize>,
    expected_revision: Option<String>,
) -> Value {
    let response = query(
        &Parameters(QueryRequest {
            path: path_of(project),
            sql: sql.to_owned(),
            offset,
            expected_revision,
            page_bytes,
        }),
        &NoopProgressSink,
        manager,
    )
    .expect("query succeeds")
    .0;
    serde_json::to_value(response).expect("response serializes")
}

fn query_case(name: &str, sql: &str, page_bytes: Option<usize>) {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    let response = run_query(&project, &manager, sql, 0, page_bytes, None);
    check_case(
        name,
        &project,
        &json!({ "sql": sql, "page_bytes": page_bytes }),
        &response,
    );
}

#[test]
fn golden_phase9_query_plain_rows() {
    query_case(
        "query_plain_rows",
        "SELECT path, language FROM files ORDER BY path",
        None,
    );
}

#[test]
fn golden_phase9_query_clipped_first_row() {
    query_case(
        "query_clipped_cells",
        "SELECT replace(hex(zeroblob(1500)), '00', 'ab') AS t, zeroblob(1500) AS b, 7 AS n",
        Some(600),
    );
}

#[test]
fn golden_phase9_query_positive_infinity() {
    query_case(
        "query_non_finite_inf",
        "SELECT 1e308*10 AS pos, -1e308*10 AS neg, 1.5 AS ok",
        None,
    );
}

#[test]
fn golden_phase9_query_nan_expressions() {
    query_case(
        "query_nan_expressions",
        "SELECT 1e308*10*0 AS mul, 1e308*10 - 1e308*10 AS sub, golden_nan() AS udf",
        None,
    );
}

#[test]
fn golden_phase9_query_text_with_nul() {
    query_case("query_text_with_nul", "SELECT 'a'||char(0)||'b' AS s", None);
}

#[test]
fn golden_phase9_query_blob() {
    query_case(
        "query_blob",
        "SELECT x'00ff' AS b, zeroblob(0) AS empty",
        None,
    );
}

#[test]
fn golden_phase9_query_numeric_looking_text() {
    query_case(
        "query_numeric_text",
        "SELECT '007' AS a, '1e3' AS b, '-0' AS c, ' 5' AS d",
        None,
    );
}

#[test]
fn golden_phase9_query_null_and_integers() {
    query_case(
        "query_null",
        "SELECT NULL AS n, 1 AS one, 0.5 AS half",
        None,
    );
    query_case(
        "query_i64_extremes",
        "SELECT 9223372036854775807 AS max_i, -9223372036854775808 AS min_i, 9007199254740993 AS past_2_53, 1e15 AS big_real",
        None,
    );
}

#[test]
fn golden_phase9_query_multi_page() {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<60) SELECT i, printf('%050d', i) AS pad FROM n";
    let one = run_query(&project, &manager, sql, 0, Some(1000), None);
    check_case(
        "query_multi_page1",
        &project,
        &json!({ "sql": sql, "page_bytes": 1000 }),
        &one,
    );
    let next = one["page"]["next_offset"]
        .as_u64()
        .expect("a second page exists");
    let revision = one["page"]["revision_id"].as_str().map(str::to_owned);
    let two = run_query(&project, &manager, sql, next as usize, Some(1000), revision);
    check_case(
        "query_multi_page2",
        &project,
        &json!({ "sql": sql, "page_bytes": 1000, "page": 2 }),
        &two,
    );
}
