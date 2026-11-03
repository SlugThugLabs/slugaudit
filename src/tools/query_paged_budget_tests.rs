//! Page-budget continuation, sync-once and marked non-finite cells on the stored path.
use super::*;

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

    ensure_eq(
        &first.page.page_bytes,
        &SMALL_PAGE,
        "first.page.page_bytes == SMALL_PAGE",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("the configured budget must page")?;
    ensure_eq(
        &second.page.page_bytes,
        &SMALL_PAGE,
        "the continuation inherits page one's budget",
    )?;
    ensure_eq(
        &second.rows.len(),
        &first.rows.len(),
        "second.rows.len() == first.rows.len()",
    )?;
    second
        .page
        .cursor
        .as_ref()
        .check("still more pages follow")?;
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

    ensure_eq(&after_first, &1, "a new request syncs exactly once")?;
    ensure_eq(
        &counter.0.load(std::sync::atomic::Ordering::SeqCst),
        &1,
        "a continuation syncs nothing",
    )?;
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

    ensure_eq(
        &collected.rows,
        &[json!({ "id": 1, "nan": null, "plain": null })],
        "collected.rows == [json!({ \"id\": 1, \"nan\": null, \"plain\": null })]",
    )?;
    ensure_eq(
        &collected.non_finite,
        &[NonFiniteCell {
            row: 0,
            column: "nan".to_owned(),
            value: "NaN",
        }],
        "collected.non_finite == [NonFiniteCell { row: 0, column: \"nan\".to_owned(), value: \"NaN\" }]",
    )?;
    Ok(())
}
