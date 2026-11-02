//! `query` on the result store: the whole result is stored once, every page
//! comes from the stored copy, and an edit between pages changes nothing the
//! reader sees. The golden fixtures under `tests/golden/phase9` pin the visible output.
use super::*;
use crate::sync::SourceSyncManager;
use crate::tools::golden_phase9_support::fixture_dir;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use slugaudit_test_check::{Check, CheckErr, TestResult};
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
    Ok(fixture["response"].take())
}

#[test]
fn paged_query_pages_survive_an_edit_between_pages() -> TestResult {
    let project = listing_project()?;
    let manager = SourceSyncManager::default();
    let limits = ResourceLimits::default();
    let full = start(&manager, &project, LISTING, HUGE_PAGE, &limits).check("full fetch")?;
    assert!(full.page.complete);
    let first = start(&manager, &project, LISTING, SMALL_PAGE, &limits).check("page one")?;
    assert!(first.page.cursor.is_some());

    fs::remove_file(project.path().join("f10.rs")).check("remove a matching file")?;
    fs::write(project.path().join("zz_new.rs"), b"pub fn z() {}\n").check("add a file")?;
    let pages = all_pages(&manager, first)?;

    assert!(pages.len() >= 3, "the fixture must page: {}", pages.len());
    assert_eq!(
        rows_of(&pages),
        full.rows,
        "the union is the page-one result"
    );
    for page in &pages {
        assert_eq!(page.page.total, Some(full.rows.len() as u64));
        assert_eq!(page.page.revision_id, full.page.revision_id);
    }
    let fresh = start(&manager, &project, LISTING, HUGE_PAGE, &limits).check("fresh request")?;
    let paths: Vec<_> = fresh.rows.iter().map(|row| row["path"].clone()).collect();
    assert!(
        !paths.contains(&json!("f10.rs")),
        "the removed file is gone"
    );
    assert!(paths.contains(&json!("zz_new.rs")), "the new file shows");
    assert_ne!(fresh.page.revision_id, full.page.revision_id);
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

    assert_eq!(
        serde_json::to_value(&one).check("serializes")?,
        serde_json::to_value(&two).check("serializes")?
    );
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
        let page = follow(&manager, &cursors[cursors.len() - 1], SMALL_PAGE).check("a page")?;
        match page.page.cursor {
            Some(next) => cursors.push(next),
            None => break cursors[cursors.len() - 1].clone(),
        }
    };

    let last = follow(&manager, &last_cursor, SMALL_PAGE).check("the final page")?;
    let again = follow(&manager, &last_cursor, SMALL_PAGE).check("the final page again")?;

    assert!(last.page.complete);
    assert_eq!(last.page.cursor, None);
    assert_eq!(
        serde_json::to_value(&last).check("serializes")?,
        serde_json::to_value(&again).check("serializes")?
    );
    Ok(())
}

#[test]
fn paged_query_refuses_offset_and_expected_revision_naming_cursor() -> TestResult {
    let project = listing_project()?;
    let path = project.path().to_string_lossy().into_owned();

    for field in ["offset", "expected_revision"] {
        let mut arguments = json!({ "path": path, "sql": "SELECT 1" });
        arguments[field] = if field == "offset" {
            json!(5)
        } else {
            json!("r1")
        };
        let request: QueryRequest = serde_json::from_value(arguments).check("still parses")?;

        let error = call(
            &SourceSyncManager::default(),
            request,
            &ResourceLimits::default(),
        )
        .check_err("a removed parameter is refused")?;

        let data = error.data.check("a payload")?;
        assert_eq!(data["code"], "paging_parameter_removed", "{field}");
        assert!(data["remedy"].as_str().check("remedy")?.contains("cursor"));
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

    assert_eq!(
        response.rows,
        [json!({ "mul": null, "sub": null, "udf": null })]
    );
    assert!(response.non_finite_cells.is_empty());
    assert_eq!(json!(response.rows), golden["rows"]);
    assert_eq!(json!(response.non_finite_cells), golden["non_finite_cells"]);
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
        assert_eq!(json!(response.rows), golden["rows"], "{name} rows");
        assert_eq!(
            json!(response.non_finite_cells),
            golden["non_finite_cells"],
            "{name} non_finite_cells"
        );
        assert_eq!(
            json!(response.clipped_cells),
            golden["clipped_cells"],
            "{name} clipped_cells"
        );
        assert_eq!(
            response.page.bytes_used, golden["page"]["bytes_used"],
            "{name}"
        );
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

    assert!(
        error.message.contains("virtual-machine step budget"),
        "{}",
        error.message
    );
    let store = manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("the store opened for the attempt")?;
    let stats = store.stats().check("read the store counters")?;
    assert_eq!(
        (
            stats.active.results,
            stats.idle.results,
            stats.delivered.results
        ),
        (0, 0, 0),
        "no partial result is left behind"
    );
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

    assert!(pages.len() > 3, "the fixture must page: {}", pages.len());
    for (number, page) in pages.iter().enumerate() {
        assert_eq!(page.page.total, Some(500), "page {number}");
    }
    assert_eq!(rows_of(&pages).len(), 500);
    Ok(())
}

/// Sets the global-config `default_page_bytes` that `resolve_limits` reads
/// (the environment is read once per process, so no env var can set it per
/// test). Keep the returned guard alive for the test.
fn write_default_page_bytes(
    page_bytes: usize,
) -> TestResult<crate::tools::test_support::GlobalConfigGuard> {
    crate::tools::test_support::use_global_config(&format!(
        "{{\"default_page_bytes\": {page_bytes}}}"
    ))
}

/// Calls the public tool entry (project limits for a new request, process
/// limits for a continuation) with no `page_bytes`.
fn call_without_page_bytes(
    manager: &SourceSyncManager,
    path: Option<String>,
    sql: &str,
    cursor: Option<String>,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<QueryResponse, ErrorData> {
    query(
        &Parameters(QueryRequest {
            path,
            sql: sql.to_owned(),
            cursor,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        sink,
        manager,
    )
    .map(|Json(response)| response)
}

#[test]
fn paged_query_continuation_keeps_the_page_budget_of_page_one() -> TestResult {
    let project = listing_project()?;
    let _config = write_default_page_bytes(SMALL_PAGE)?;
    let manager = SourceSyncManager::default();
    let path = project.path().to_string_lossy().into_owned();
    let noop = crate::progress::NoopProgressSink;

    let first =
        call_without_page_bytes(&manager, Some(path), LISTING, None, &noop).check("page one")?;
    let second = call_without_page_bytes(&manager, None, "", first.page.cursor.clone(), &noop)
        .check("page two")?;

    assert_eq!(first.page.page_bytes, SMALL_PAGE);
    assert!(
        first.page.cursor.is_some(),
        "the configured budget must page"
    );
    assert_eq!(
        second.page.page_bytes, SMALL_PAGE,
        "the continuation inherits page one's budget"
    );
    assert_eq!(second.rows.len(), first.rows.len());
    assert!(second.page.cursor.is_some(), "still more pages follow");
    Ok(())
}

/// Counts how many times the sync layer began an `ensure_current`.
#[derive(Default)]
struct SyncCounter(std::sync::atomic::AtomicUsize);

impl crate::progress::ProgressSink for SyncCounter {
    fn emit(&self, event: crate::progress::ProgressEvent) {
        if event
            == (crate::progress::ProgressEvent::Started {
                phase: "ensuring_current",
            })
        {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[test]
fn paged_query_new_request_syncs_once() -> TestResult {
    let project = listing_project()?;
    let _config = write_default_page_bytes(SMALL_PAGE)?;
    let manager = SourceSyncManager::default();
    let path = project.path().to_string_lossy().into_owned();
    let counter = SyncCounter::default();

    let first = call_without_page_bytes(&manager, Some(path), LISTING, None, &counter)
        .check("a new request")?;
    let after_first = counter.0.load(std::sync::atomic::Ordering::SeqCst);
    call_without_page_bytes(&manager, None, "", first.page.cursor, &counter)
        .check("a continuation")?;

    assert_eq!(after_first, 1, "a new request syncs exactly once");
    assert_eq!(
        counter.0.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a continuation syncs nothing"
    );
    Ok(())
}

/// The NaN restore branch of `values_of` (a NULL cell carrying the `nonfinite`
/// NaN mark becomes a REAL NaN again) cannot be reached from SQL in this
/// bundled build: SQLite turns a NaN into NULL before the cell is stored, so
/// the query path never writes a mark. The mark is therefore planted through
/// the store API, and the tool layer must still list the cell as `NaN` while
/// rendering it as JSON `null`; an unmarked NULL beside it stays plain NULL.
#[test]
fn paged_query_marked_nan_cell_is_restored_and_listed_as_non_finite() -> TestResult {
    use crate::store::{Cell, ResultSpec};
    use crate::tools::query_value::{NonFiniteCell, PageBuilder};
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    let store = manager
        .result_stores()
        .for_dir(&project.path().join(".planning").join("slugaudit"))
        .check("open the result store")?;
    let names = vec!["id".to_owned(), "nan".to_owned(), "plain".to_owned()];
    let mut writer = store
        .begin(ResultSpec {
            tool: "query".to_owned(),
            project: "proj".to_owned(),
            revision_id: "rev-1".to_owned(),
            columns: names.clone(),
        })
        .check("begin the result")?;
    writer
        .push(&[Cell::Int(1), Cell::Real(f64::NAN), Cell::Null])
        .check("push the NaN row")?;
    let handle = writer
        .finish(&json!({"kind": "complete"}))
        .check("finish the result")?;
    let mut builder = PageBuilder::new();
    builder.set_names(names);
    builder.set_budget(65_536);

    let page = store
        .read_page(handle.id, 0, |row| builder.accept(row))
        .check("read the page")?;
    let collected = builder.finish(page.rows.len()).check("build the page")?;

    assert_eq!(
        collected.rows,
        [json!({ "id": 1, "nan": null, "plain": null })]
    );
    assert_eq!(
        collected.non_finite,
        [NonFiniteCell {
            row: 0,
            column: "nan".to_owned(),
            value: "NaN"
        }]
    );
    Ok(())
}
