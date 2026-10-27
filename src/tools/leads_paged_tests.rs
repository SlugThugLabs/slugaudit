//! Paging `leads` through the result store: every page of a result describes
//! the project as page one saw it, whatever happens to the tree meanwhile.

use super::*;
use crate::tools::context::{ensure_synced, with_verified_read};

/// A page size that forces several pages out of `unwrap_fixture(4, 6)`.
const SMALL_PAGE: usize = 400;

fn places(response: &LeadsResponse) -> Vec<(String, i64)> {
    full_rows(response)
        .iter()
        .map(|row| (row.file.clone(), row.line))
        .collect()
}

/// Reads every page after `first`, following the cursors, and returns them.
fn rest_of(manager: &crate::sync::SourceSyncManager, first: &LeadsResponse) -> Vec<LeadsResponse> {
    rest_of_limited(manager, first, None)
}

/// [`rest_of`] repeating the `limit` the first page was asked for.
fn rest_of_limited(
    manager: &crate::sync::SourceSyncManager,
    first: &LeadsResponse,
    limit: Option<usize>,
) -> Vec<LeadsResponse> {
    let mut pages: Vec<LeadsResponse> = Vec::new();
    while let Some(last) = pages.last().or(Some(first)) {
        if last.page.cursor.is_none() {
            break;
        }
        let mut next = continuation(last);
        next.limit = limit;
        let page = run_on(manager, next);
        pages.push(page);
    }
    pages
}

fn small_pages(project: &tempfile::TempDir) -> LeadsRequest {
    let mut first = request(project.path(), Some("unwrap_calls"));
    first.page_bytes = Some(SMALL_PAGE);
    first
}

#[test]
fn paged_leads_pages_survive_an_edit_between_pages() {
    let project = unwrap_fixture(4, 6); // 24 rows
    let manager = crate::sync::SourceSyncManager::default();
    let mut whole = request(project.path(), Some("unwrap_calls"));
    whole.page_bytes = Some(1_000_000);
    let whole = run_on(&manager, whole);
    assert_eq!(whole.page.total, Some(24));
    let first = run_on(&manager, small_pages(&project));
    assert!(
        first.page.cursor.is_some(),
        "the first page is not the last"
    );

    // One match vanishes (mod0 loses a function), one appears (a new file).
    let shorter: String = (0..5)
        .map(|n| format!("pub fn f{n}(x: Option<u8>) -> u8 {{ x.unwrap() }}\n"))
        .collect();
    fs::write(project.path().join("mod0.rs"), shorter).expect("shrink mod0");
    fs::write(
        project.path().join("extra.rs"),
        b"pub fn e(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )
    .expect("add a file");
    let rest = rest_of(&manager, &first);

    assert!(
        rest.len() >= 2,
        "at least three pages in all: {}",
        rest.len()
    );
    let mut union = places(&first);
    for page in &rest {
        assert_eq!(page.page.revision_id, first.page.revision_id);
        assert_eq!(page.page.total, Some(24), "the total never drifts");
        union.extend(places(page));
    }
    assert_eq!(union, places(&whole), "the union is the fetch at page one");
    assert!(rest.last().expect("a last page").page.complete);

    let fresh = run_on(&manager, small_pages(&project));
    assert_ne!(fresh.page.revision_id, first.page.revision_id);
    assert_eq!(fresh.page.total, Some(24), "one gone and one new");
    let mut fresh_places = places(&fresh);
    for page in rest_of(&manager, &fresh) {
        fresh_places.extend(places(&page));
    }
    assert!(fresh_places.iter().any(|(file, _)| file == "extra.rs"));
    assert!(!fresh_places.contains(&("mod0.rs".to_owned(), 6)));
    assert!(union.contains(&("mod0.rs".to_owned(), 6)));
}

#[test]
fn paged_leads_same_cursor_twice_returns_identical_pages() {
    let project = unwrap_fixture(4, 6);
    let manager = crate::sync::SourceSyncManager::default();
    let first = run_on(&manager, small_pages(&project));

    let once = run_on(&manager, continuation(&first));
    let twice = run_on(&manager, continuation(&first));

    assert_eq!(
        serde_json::to_value(&once).expect("serializes"),
        serde_json::to_value(&twice).expect("serializes")
    );
    assert!(once.page.returned > 0);
}

#[test]
fn paged_leads_final_page_can_be_retried() {
    let project = unwrap_fixture(4, 6);
    let manager = crate::sync::SourceSyncManager::default();
    let first = run_on(&manager, small_pages(&project));
    let mut penultimate = first;
    let last = loop {
        let next = run_on(&manager, continuation(&penultimate));
        if next.page.complete {
            break next;
        }
        penultimate = next;
    };

    let retried = run_on(&manager, continuation(&penultimate));

    assert!(last.page.complete && last.page.cursor.is_none());
    assert_eq!(
        serde_json::to_value(&retried).expect("serializes"),
        serde_json::to_value(&last).expect("serializes")
    );
}

#[test]
fn paged_leads_refuses_offset_and_expected_revision_naming_cursor() {
    let project = unwrap_fixture(1, 1);
    let path = project.path().to_string_lossy().into_owned();
    for field in [
        serde_json::json!({ "path": path, "offset": 2 }),
        serde_json::json!({ "path": path, "expected_revision": "r1" }),
    ] {
        let request: LeadsRequest = serde_json::from_value(field).expect("the field is accepted");

        let error = try_run(request).expect_err("a removed parameter is refused");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let data = error.data.expect("a payload");
        assert_eq!(data["code"], "paging_parameter_removed");
        assert!(data["remedy"].as_str().expect("remedy").contains("cursor"));
    }
}

/// Counts and rows are produced by one closure inside one verified read
/// transaction (`leads_page::produce`), so an edit cannot land between the
/// two. This asserts what that buys: every page reports the counts of page
/// one, and the counts agree with the stored rows (`total`). It also shows
/// the transaction property itself: a publish that commits between a count
/// read and a row read of one transaction is invisible to both.
#[test]
fn paged_leads_counts_and_rows_come_from_one_revision() {
    let project = unwrap_fixture(3, 4);
    let manager = crate::sync::SourceSyncManager::default();
    let path = project.path().to_string_lossy().into_owned();
    let synced = ensure_synced(&path, &crate::progress::NoopProgressSink, &manager).expect("sync");
    let (counted, read) = with_verified_read(&synced, |tx| {
        let languages = crate::tools::leads_covers::languages_read(tx, None)?;
        let counts = crate::tools::leads_query::count_all(tx, None, &languages)?;
        fs::write(
            project.path().join("late.rs"),
            b"pub fn l(x: Option<u8>) -> u8 { x.unwrap() }\n",
        )
        .expect("edit mid-transaction");
        let other = crate::sync::SourceSyncManager::default();
        ensure_synced(&path, &crate::progress::NoopProgressSink, &other)
            .expect("the edit publishes a new revision");
        let mut rows = 0_i64;
        let unwrap = &crate::tools::leads_mechanics::MECHANICS[0];
        crate::tools::leads_query::stream_rows(tx, unwrap, None, &mut |_| {
            rows += 1;
            Ok(())
        })?;
        Ok((counts[0].count, rows))
    })
    .expect("the transaction reads one snapshot");
    assert_eq!(counted, read, "a mid-transaction publish is invisible");

    let first = run_on(&manager, small_pages(&project));
    fs::write(
        project.path().join("later.rs"),
        b"pub fn l(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )
    .expect("edit after page one");
    let rest = rest_of(&manager, &first);
    for page in std::iter::once(&first).chain(&rest) {
        assert_eq!(count_of(page, "unwrap_calls"), 13);
        assert_eq!(page.page.total, Some(13));
        assert_eq!(page.files_total, 4);
    }
}

#[test]
fn paged_leads_orders_equal_lines_by_id() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    let body = "pub fn f(a: Option<u8>, b: Option<u8>, c: Option<u8>) -> u8 { c.unwrap() + a.unwrap() + b.unwrap() }\n";
    fs::write(project.path().join("lib.rs"), body).expect("write source");
    let manager = crate::sync::SourceSyncManager::default();
    let path = project.path().to_string_lossy().into_owned();
    let synced = ensure_synced(&path, &crate::progress::NoopProgressSink, &manager).expect("sync");
    let connection = crate::store::open_read_only(&synced.database_path).expect("open the index");
    let mut statement = connection
        .prepare(
            "SELECT json_extract(e.payload, '$.callee_text') FROM evidence e \
             WHERE e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') = 'unwrap' \
             ORDER BY e.id",
        )
        .expect("prepare");
    let by_id: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("rows");

    let response = run_on(&manager, request(project.path(), Some("unwrap_calls")));

    let details: Vec<&str> = full_rows(&response)
        .iter()
        .map(|row| row.detail.as_str())
        .collect();
    assert_eq!(details.len(), 3);
    assert!(full_rows(&response).iter().all(|row| row.line == 1));
    assert_eq!(
        details,
        by_id.iter().map(String::as_str).collect::<Vec<_>>()
    );
    let sql = crate::tools::leads_mechanics::MECHANICS[0].order_expr();
    assert_eq!(sql, "f.path, e.start_line, e.id");
}

fn eval_entry() -> crate::project::MechanicEntry {
    crate::project::MechanicEntry {
        id: "py_eval".to_owned(),
        language: "python".to_owned(),
        query: r#"(call function: (identifier) @callee (#eq? @callee "eval"))"#.to_owned(),
        note: "dynamic code execution".to_owned(),
    }
}

#[test]
fn paged_leads_custom_scan_stopped_by_deadline_is_partial_on_every_page() {
    let project = unwrap_fixture(3, 4);
    fs::write(project.path().join("app.py"), "eval(\"1\")\n").expect("write python");
    let path = project.path().to_string_lossy().into_owned();
    let manager = crate::sync::SourceSyncManager::default();
    manager
        .session_mechanics()
        .with_entries(|entries| entries.push(eval_entry()));
    let limits = ResourceLimits {
        max_structure_execution_time: std::time::Duration::ZERO,
        ..ResourceLimits::default()
    };
    let root = crate::tools::project_root(&path).expect("root");
    let catalog = mechanic_catalog::load(&root, manager.session_mechanics(), &limits);
    let mut whole_surface = compact_request(project.path(), None);
    whole_surface.limit = Some(5);

    let first = leads_with_catalog(
        &whole_surface,
        &limits,
        &catalog,
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("a stopped scan is a partial result");

    let expected = Some(crate::tools::page::Completeness::Partial {
        limited_by: PageLimit::TimeBudget,
        resume_after_file: None,
        files_scanned: 0,
        files_total: 1,
    });
    assert_eq!(first.page.completeness, expected);
    let rest = rest_of_limited(&manager, &first, Some(5));
    assert!(!rest.is_empty(), "the built-in rows still page");
    for page in &rest {
        assert_eq!(page.page.completeness, expected, "every page says so");
    }
}

#[test]
fn paged_leads_compact_pages_by_cursor() {
    let project = unwrap_fixture(3, 4); // 12 unwraps across 3 files
    let manager = crate::sync::SourceSyncManager::default();
    let mut whole = compact_request(project.path(), None);
    whole.page_bytes = Some(1_000_000);
    let whole = run_on(&manager, whole);
    let mut first = compact_request(project.path(), None);
    first.limit = Some(5);
    let first = run_on(&manager, first);

    let rest = rest_of_limited(&manager, &first, Some(5));

    let key = |row: &CompactLeadRow| (row.mechanic.clone(), row.file.clone(), row.line);
    let mut union: Vec<_> = compact_rows(&first).iter().map(key).collect();
    for page in &rest {
        union.extend(compact_rows(page).iter().map(key));
    }
    let expected: Vec<_> = compact_rows(&whole).iter().map(key).collect();
    assert_eq!(union, expected);
    assert_eq!(rest.len(), 2, "12 rows at 5 per page");
    let covered: Vec<i64> = std::iter::once(&first)
        .chain(&rest)
        .map(|page| page.files_covered)
        .collect();
    assert!(
        covered.windows(2).all(|pair| pair[0] <= pair[1]),
        "{covered:?}"
    );
    assert_eq!(covered.last().copied(), Some(first.files_total));
    assert!(rest.last().expect("a last page").page.complete);
}
