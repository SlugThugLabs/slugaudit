//! `query` on the result store: the whole result is stored once, every page
//! comes from the stored copy, and an edit between pages changes nothing the
//! reader sees. The golden fixtures under `tests/golden/phase9` pin the visible output.
use super::*;
use crate::sync::SourceSyncManager;
use crate::tools::golden_phase9_support::fixture_dir;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::fs;

/// 60 files, so a small page forces several pages: one row each.
const LISTING: &str = "SELECT path, hex(zeroblob(8)) AS pad FROM files ORDER BY path";
const SMALL_PAGE: usize = 400;
const HUGE_PAGE: usize = 10_000_000;

fn listing_project() -> TestResult<tempfile::TempDir> {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    for n in 0..60 {
        fs::write(
            project.path().join(format!("f{n:02}.rs")),
            b"pub fn a() {}\n",
        )
        .check("write fixture file")?;
    }
    Ok(project)
}

fn start(
    manager: &SourceSyncManager,
    project: &tempfile::TempDir,
    sql: &str,
    page_bytes: usize,
    limits: &ResourceLimits,
) -> Result<QueryResponse, ErrorData> {
    call(
        manager,
        QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: Some(page_bytes),
        },
        limits,
    )
}

fn follow(
    manager: &SourceSyncManager,
    cursor: &str,
    page_bytes: usize,
) -> Result<QueryResponse, ErrorData> {
    call(
        manager,
        QueryRequest {
            path: None,
            sql: String::new(),
            cursor: Some(cursor.to_owned()),
            legacy_paging: LegacyPaging::default(),
            page_bytes: Some(page_bytes),
        },
        &ResourceLimits::default(),
    )
}

fn call(
    manager: &SourceSyncManager,
    request: QueryRequest,
    limits: &ResourceLimits,
) -> Result<QueryResponse, ErrorData> {
    query_with_limits(
        &Parameters(request),
        limits,
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
}

/// Every page of a result, first to last, following the cursors.
fn all_pages(manager: &SourceSyncManager, first: QueryResponse) -> TestResult<Vec<QueryResponse>> {
    let mut pages = vec![first];
    while let Some(cursor) = pages.last().and_then(|page| page.page.cursor.clone()) {
        pages.push(follow(manager, &cursor, SMALL_PAGE).check("a later page")?);
    }
    Ok(pages)
}

fn rows_of(pages: &[QueryResponse]) -> Vec<Value> {
    pages
        .iter()
        .flat_map(|page| page.rows.iter().cloned())
        .collect()
}

fn fixture_response(name: &str) -> TestResult<Value> {
    let text = fs::read_to_string(fixture_dir().join(format!("{name}.json")))
        .check("the golden fixture exists")?;
    let mut fixture: Value = serde_json::from_str(&text).check("the fixture is JSON")?;
    Ok(fixture
        .get_mut("response")
        .check("fixture[\"response\"]")?
        .take())
}

#[test]
fn paged_query_pages_survive_an_edit_between_pages() -> TestResult {
    let project = listing_project()?;
    let manager = SourceSyncManager::default();
    let limits = ResourceLimits::default();
    let full = start(&manager, &project, LISTING, HUGE_PAGE, &limits).check("full fetch")?;
    ensure(full.page.complete, "full.page.complete")?;
    let first = start(&manager, &project, LISTING, SMALL_PAGE, &limits).check("page one")?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;

    fs::remove_file(project.path().join("f10.rs")).check("remove a matching file")?;
    fs::write(project.path().join("zz_new.rs"), b"pub fn z() {}\n").check("add a file")?;
    let pages = all_pages(&manager, first)?;

    ensure(
        pages.len() >= 3,
        format!("the fixture must page: {}", pages.len()),
    )?;
    ensure_eq(
        &rows_of(&pages),
        &full.rows,
        "the union is the page-one result",
    )?;
    for page in &pages {
        ensure_eq(
            &page.page.total,
            &Some(full.rows.len() as u64),
            "page.page.total == Some(full.rows.len() as u64)",
        )?;
        ensure_eq(
            &page.page.revision_id,
            &full.page.revision_id,
            "page.page.revision_id == full.page.revision_id",
        )?;
    }
    let fresh = start(&manager, &project, LISTING, HUGE_PAGE, &limits).check("fresh request")?;
    let paths: Vec<_> = fresh.rows.iter().map(|row| row["path"].clone()).collect();
    ensure(
        !paths.contains(&json!("f10.rs")),
        "the removed file is gone",
    )?;
    ensure(paths.contains(&json!("zz_new.rs")), "the new file shows")?;
    ensure_ne(
        &fresh.page.revision_id,
        &full.page.revision_id,
        "fresh.page.revision_id != full.page.revision_id",
    )?;
    Ok(())
}

#[test]
fn paged_query_same_cursor_twice_returns_identical_pages() -> TestResult {
    let project = listing_project()?;
    let manager = SourceSyncManager::default();
    let first = start(
        &manager,
        &project,
        LISTING,
        SMALL_PAGE,
        &ResourceLimits::default(),
    )
    .check("page one")?;
    let cursor = first.page.cursor.check("more pages follow")?;

    let one = follow(&manager, &cursor, SMALL_PAGE).check("first read")?;
    let two = follow(&manager, &cursor, SMALL_PAGE).check("second read")?;

    ensure_eq(
        &serde_json::to_value(&one).check("serializes")?,
        &serde_json::to_value(&two).check("serializes")?,
        "serde_json::to_value(&one).check(\"serializes\")? == serde_json::to_value(&two).check(\"serializes\")?",
    )?;
    Ok(())
}

#[test]
fn paged_query_final_page_can_be_retried() -> TestResult {
    let project = listing_project()?;
    let manager = SourceSyncManager::default();
    let first = start(
        &manager,
        &project,
        LISTING,
        SMALL_PAGE,
        &ResourceLimits::default(),
    )
    .check("page one")?;
    let mut cursors = vec![first.page.cursor.clone().check("more pages follow")?];
    let last_cursor = loop {
        let newest = cursors.last().check("a cursor to follow")?;
        let page = follow(&manager, newest, SMALL_PAGE).check("a page")?;
        match page.page.cursor {
            Some(next) => cursors.push(next),
            None => break cursors.last().check("the last cursor")?.clone(),
        }
    };

    let last = follow(&manager, &last_cursor, SMALL_PAGE).check("the final page")?;
    let again = follow(&manager, &last_cursor, SMALL_PAGE).check("the final page again")?;

    ensure(last.page.complete, "last.page.complete")?;
    ensure_eq(&last.page.cursor, &None, "last.page.cursor == None")?;
    ensure_eq(
        &serde_json::to_value(&last).check("serializes")?,
        &serde_json::to_value(&again).check("serializes")?,
        "serde_json::to_value(&last).check(\"serializes\")? == serde_json::to_value(&again).check(\"serializes\")?",
    )?;
    Ok(())
}

#[test]
fn paged_query_refuses_offset_and_expected_revision_naming_cursor() -> TestResult {
    let project = listing_project()?;
    let path = project.path().to_string_lossy().into_owned();

    for field in ["offset", "expected_revision"] {
        let mut arguments = json!({ "path": path, "sql": "SELECT 1" });
        let value = if field == "offset" {
            json!(5)
        } else {
            json!("r1")
        };
        arguments
            .as_object_mut()
            .check("the arguments are an object")?
            .insert(field.to_owned(), value);
        let request: QueryRequest = serde_json::from_value(arguments).check("still parses")?;

        let error = call(
            &SourceSyncManager::default(),
            request,
            &ResourceLimits::default(),
        )
        .check_err("a removed parameter is refused")?;

        let data = error.data.check("a payload")?;
        ensure_eq(
            &data.get("code").check("data[\"code\"]")?,
            &"paging_parameter_removed",
            field,
        )?;
        ensure(
            data.get("remedy")
                .check("data[\"remedy\"]")?
                .as_str()
                .check("remedy")?
                .contains("cursor"),
            "data[\"remedy\"].as_str().check(\"remedy\")?.contains(\"cursor\")",
        )?;
    }
    Ok(())
}

#[test]
fn paged_query_nan_expression_returns_null_with_no_non_finite_cells_on_the_store_path() -> TestResult
{
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    let golden = fixture_response("query_nan_expressions")?;
    let sql = "SELECT 1e308*10*0 AS mul, 1e308*10 - 1e308*10 AS sub, golden_nan() AS udf";

    let response = start(&manager, &project, sql, 65_536, &ResourceLimits::default())
        .check("query succeeds")?;

    ensure_eq(
        &response.rows,
        &[json!({ "mul": null, "sub": null, "udf": null })],
        "response.rows == [json!({ \"mul\": null, \"sub\": null, \"udf\": null })]",
    )?;
    ensure(
        response.non_finite_cells.is_empty(),
        "response.non_finite_cells.is_empty()",
    )?;
    ensure_eq(
        &json!(response.rows),
        golden.get("rows").check("golden[\"rows\"]")?,
        "json!(response.rows) == golden[\"rows\"]",
    )?;
    ensure_eq(
        &json!(response.non_finite_cells),
        golden
            .get("non_finite_cells")
            .check("golden[\"non_finite_cells\"]")?,
        "json!(response.non_finite_cells) == golden[\"non_finite_cells\"]",
    )?;
    Ok(())
}

#[test]
fn paged_query_infinity_nul_text_blob_numeric_text_equal_the_golden() -> TestResult {
    let manager = SourceSyncManager::default();
    let cases = [
        (
            "query_non_finite_inf",
            "SELECT 1e308*10 AS pos, -1e308*10 AS neg, 1.5 AS ok",
        ),
        ("query_text_with_nul", "SELECT 'a'||char(0)||'b' AS s"),
        ("query_blob", "SELECT x'00ff' AS b, zeroblob(0) AS empty"),
        (
            "query_numeric_text",
            "SELECT '007' AS a, '1e3' AS b, '-0' AS c, ' 5' AS d",
        ),
    ];

    for (name, sql) in cases {
        let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
        let response = start(&manager, &project, sql, 65_536, &ResourceLimits::default())
            .check("query succeeds")?;

        let golden = fixture_response(name)?;
        ensure_eq(
            &json!(response.rows),
            golden.get("rows").check("golden[\"rows\"]")?,
            format!("{name} rows"),
        )?;
        ensure_eq(
            &json!(response.non_finite_cells),
            golden
                .get("non_finite_cells")
                .check("golden[\"non_finite_cells\"]")?,
            format!("{name} non_finite_cells"),
        )?;
        ensure_eq(
            &json!(response.clipped_cells),
            golden
                .get("clipped_cells")
                .check("golden[\"clipped_cells\"]")?,
            format!("{name} clipped_cells"),
        )?;
        ensure_eq(
            &response.page.bytes_used,
            golden
                .get("page")
                .check("golden[\"page\"]")?
                .get("bytes_used")
                .check("golden[\"page\"][\"bytes_used\"]")?,
            name,
        )?;
    }
    Ok(())
}

#[test]
fn paged_query_over_vm_step_budget_leaves_no_cursor_and_no_stored_result() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    let limits = ResourceLimits {
        max_query_vm_steps: 20_000,
        ..ResourceLimits::default()
    };
    let sql = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 200000) \
         SELECT x FROM c";

    let error = start(&manager, &project, sql, 40, &limits)
        .check_err("a spent budget fails the whole call")?;

    ensure(
        error.message.contains("virtual-machine step budget"),
        format!("{}", error.message),
    )?;
    let store = manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("the store opened for the attempt")?;
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
    Ok(())
}

#[test]
fn paged_query_total_is_exact_on_every_page() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    let sql = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 500) \
         SELECT x, hex(zeroblob(500)) AS pad FROM c";
    let first =
        start(&manager, &project, sql, 20_000, &ResourceLimits::default()).check("page one")?;

    let pages = all_pages(&manager, first)?;

    ensure(
        pages.len() > 3,
        format!("the fixture must page: {}", pages.len()),
    )?;
    for (number, page) in pages.iter().enumerate() {
        ensure_eq(&page.page.total, &Some(500), format!("page {number}"))?;
    }
    ensure_eq(&rows_of(&pages).len(), &500, "rows_of(&pages).len() == 500")?;
    Ok(())
}

#[path = "query_paged_budget_tests.rs"]
mod budget_tests;
