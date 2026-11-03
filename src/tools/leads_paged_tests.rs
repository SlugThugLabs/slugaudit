//! Paging `leads` through the result store: every page of a result describes
//! the project as page one saw it, whatever happens to the tree meanwhile.

use super::*;
use crate::tools::context::{ensure_synced, with_verified_read};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};

/// A page size that forces several pages out of `marker_fixture(4, 6)`.
const SMALL_PAGE: usize = 400;

fn places(response: &LeadsResponse) -> TestResult<Vec<(String, i64)>> {
    Ok(full_rows(response)?
        .iter()
        .map(|row| (row.file.clone(), row.line))
        .collect())
}

/// Reads every page after `first`, following the cursors, and returns them.
fn rest_of(
    manager: &crate::sync::SourceSyncManager,
    first: &LeadsResponse,
) -> TestResult<Vec<LeadsResponse>> {
    rest_of_limited(manager, first, None)
}

/// [`rest_of`] repeating the `limit` the first page was asked for.
fn rest_of_limited(
    manager: &crate::sync::SourceSyncManager,
    first: &LeadsResponse,
    limit: Option<usize>,
) -> TestResult<Vec<LeadsResponse>> {
    let mut pages: Vec<LeadsResponse> = Vec::new();
    while let Some(last) = pages.last().or(Some(first)) {
        if last.page.cursor.is_none() {
            break;
        }
        let mut next = continuation(last)?;
        next.limit = limit;
        let page = run_on(manager, next)?;
        pages.push(page);
    }
    Ok(pages)
}

fn small_pages(project: &tempfile::TempDir) -> LeadsRequest {
    let mut first = request(project.path(), Some("comment_markers"));
    first.page_bytes = Some(SMALL_PAGE);
    first
}

#[test]
fn paged_leads_pages_survive_an_edit_between_pages() -> TestResult {
    let project = marker_fixture(4, 6)?; // 24 rows
    let manager = crate::sync::SourceSyncManager::default();
    let mut whole = request(project.path(), Some("comment_markers"));
    whole.page_bytes = Some(1_000_000);
    let whole = run_on(&manager, whole)?;
    ensure_eq(&whole.page.total, &Some(24), "whole.page.total == Some(24)")?;
    let first = run_on(&manager, small_pages(&project))?;
    first
        .page
        .cursor
        .as_ref()
        .check("the first page is not the last")?;

    // One match vanishes (mod0 loses a function), one appears (a new file).
    let shorter = (0..5)
        .map(|n| format!("// TODO: {n}\npub fn f{n}() {{}}\n"))
        .collect::<Vec<_>>()
        .concat();
    fs::write(project.path().join("mod0.rs"), shorter).check("shrink mod0")?;
    fs::write(
        project.path().join("extra.rs"),
        b"// TODO: e\npub fn e() {}\n",
    )
    .check("add a file")?;
    let rest = rest_of(&manager, &first)?;

    ensure(
        rest.len() >= 2,
        format!("at least three pages in all: {}", rest.len()),
    )?;
    let mut union = places(&first)?;
    for page in &rest {
        ensure_eq(
            &page.page.revision_id,
            &first.page.revision_id,
            "page.page.revision_id == first.page.revision_id",
        )?;
        ensure_eq(&page.page.total, &Some(24), "page.page.total == Some(24)")?;
        union.extend(places(page)?);
    }
    ensure_eq(&union, &places(&whole)?, "union == places(&whole)?")?;
    ensure(
        rest.last().check("a last page")?.page.complete,
        "rest.last().check(\"a last page\")?.page.complete",
    )?;

    let fresh = run_on(&manager, small_pages(&project))?;
    ensure_ne(
        &fresh.page.revision_id,
        &first.page.revision_id,
        "fresh.page.revision_id != first.page.revision_id",
    )?;
    ensure_eq(&fresh.page.total, &Some(24), "fresh.page.total == Some(24)")?;
    let mut fresh_places = places(&fresh)?;
    for page in rest_of(&manager, &fresh)? {
        fresh_places.extend(places(&page)?);
    }
    ensure(
        fresh_places.iter().any(|(file, _)| file == "extra.rs"),
        "fresh_places.iter().any(|(file, _)| file == \"extra.rs\")",
    )?;
    // The sixth marker of mod0 sits on line 11; the shortened file has five.
    ensure(
        !fresh_places.contains(&("mod0.rs".to_owned(), 11)),
        "!fresh_places.contains(&(\"mod0.rs\".to_owned(), 11))",
    )?;
    ensure(
        union.contains(&("mod0.rs".to_owned(), 11)),
        "union.contains(&(\"mod0.rs\".to_owned(), 11))",
    )?;
    Ok(())
}

#[test]
fn paged_leads_same_cursor_twice_returns_identical_pages() -> TestResult {
    let project = marker_fixture(4, 6)?;
    let manager = crate::sync::SourceSyncManager::default();
    let first = run_on(&manager, small_pages(&project))?;

    let once = run_on(&manager, continuation(&first)?)?;
    let twice = run_on(&manager, continuation(&first)?)?;

    ensure_eq(
        &serde_json::to_value(&once).check("serializes")?,
        &serde_json::to_value(&twice).check("serializes")?,
        "serde_json::to_value(&once).check(\"serializes\")? == serde_json::to_value(&twice).check(\"serializes\")?",
    )?;
    ensure(once.page.returned > 0, "once.page.returned > 0")?;
    Ok(())
}

#[test]
fn paged_leads_final_page_can_be_retried() -> TestResult {
    let project = marker_fixture(4, 6)?;
    let manager = crate::sync::SourceSyncManager::default();
    let first = run_on(&manager, small_pages(&project))?;
    let mut penultimate = first;
    let last = loop {
        let next = run_on(&manager, continuation(&penultimate)?)?;
        if next.page.complete {
            break next;
        }
        penultimate = next;
    };

    let retried = run_on(&manager, continuation(&penultimate)?)?;

    ensure(
        last.page.complete && last.page.cursor.is_none(),
        "last.page.complete && last.page.cursor.is_none()",
    )?;
    ensure_eq(
        &serde_json::to_value(&retried).check("serializes")?,
        &serde_json::to_value(&last).check("serializes")?,
        "serde_json::to_value(&retried).check(\"serializes\")? == serde_json::to_value(&last).check(\"serializes\")?",
    )?;
    Ok(())
}

#[test]
fn paged_leads_refuses_offset_and_expected_revision_naming_cursor() -> TestResult {
    let project = marker_fixture(1, 1)?;
    let path = project.path().to_string_lossy().into_owned();
    for field in [
        serde_json::json!({ "path": path, "offset": 2 }),
        serde_json::json!({ "path": path, "expected_revision": "r1" }),
    ] {
        let request: LeadsRequest = serde_json::from_value(field).check("the field is accepted")?;

        let error = try_run(request).check_err("a removed parameter is refused")?;

        ensure_eq(
            &error.code,
            &rmcp::model::ErrorCode::INVALID_PARAMS,
            "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
        )?;
        let data = error.data.check("a payload")?;
        ensure_eq(
            data.get("code").check("data[\"code\"]")?,
            &"paging_parameter_removed",
            "data[\"code\"] == \"paging_parameter_removed\"",
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

/// Counts and rows are produced by one closure inside one verified read
/// transaction (`leads_page::produce`), so an edit cannot land between the
/// two. This asserts what that buys: every page reports the counts of page
/// one, and the counts agree with the stored rows (`total`). It also shows
/// the transaction property itself: a publish that commits between a count
/// read and a row read of one transaction is invisible to both.
#[test]
fn paged_leads_counts_and_rows_come_from_one_revision() -> TestResult {
    let project = marker_fixture(3, 4)?;
    let manager = crate::sync::SourceSyncManager::default();
    let path = project.path().to_string_lossy().into_owned();
    let synced =
        ensure_synced(&path, &crate::progress::NoopProgressSink, &manager).check("sync")?;
    let (counted, read) = with_verified_read(&synced, |tx| {
        let languages = crate::tools::leads_covers::languages_read(tx, None)?;
        let counts = crate::tools::leads_query::count_all(tx, None, &languages)?;
        fs::write(
            project.path().join("late.rs"),
            b"// TODO: l\npub fn l() {}\n",
        )
        .map_err(|error| {
            ErrorData::internal_error(format!("edit mid-transaction: {error}"), None)
        })?;
        let other = crate::sync::SourceSyncManager::default();
        ensure_synced(&path, &crate::progress::NoopProgressSink, &other).map_err(|error| {
            ErrorData::internal_error(
                format!("the edit publishes a new revision: {}", error.message),
                None,
            )
        })?;
        let mut rows = 0_i64;
        let markers = crate::tools::leads_mechanics::MECHANICS
            .iter()
            .find(|mechanic| mechanic.id == "comment_markers")
            .ok_or_else(|| ErrorData::internal_error("comment_markers is built in", None))?;
        crate::tools::leads_query::stream_rows(tx, markers, None, &mut |_| {
            rows += 1;
            Ok(())
        })?;
        let counted = counts
            .iter()
            .find(|count| count.mechanic == "comment_markers")
            .ok_or_else(|| ErrorData::internal_error("comment_markers is counted", None))?;
        Ok((counted.count, rows))
    })
    .check("the transaction reads one snapshot")?;
    ensure_eq(&counted, &read, "counted == read")?;

    let first = run_on(&manager, small_pages(&project))?;
    fs::write(
        project.path().join("later.rs"),
        b"// TODO: l\npub fn l() {}\n",
    )
    .check("edit after page one")?;
    let rest = rest_of(&manager, &first)?;
    for page in std::iter::once(&first).chain(&rest) {
        ensure_eq(
            &count_of(page, "comment_markers")?,
            &13,
            "count_of(page, \"comment_markers\")? == 13",
        )?;
        ensure_eq(&page.page.total, &Some(13), "page.page.total == Some(13)")?;
        ensure_eq(&page.files_total, &4, "page.files_total == 4")?;
    }
    Ok(())
}

#[test]
fn paged_leads_orders_equal_lines_by_id() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    // Three markers on one line, written out of alphabetical order so the
    // tie-break (id, i.e. source order) is distinguishable from a text sort.
    let body = "/* TODO: c */ /* TODO: a */ /* TODO: b */ pub fn f() {}\n";
    fs::write(project.path().join("lib.rs"), body).check("write source")?;
    let manager = crate::sync::SourceSyncManager::default();
    let path = project.path().to_string_lossy().into_owned();
    let synced =
        ensure_synced(&path, &crate::progress::NoopProgressSink, &manager).check("sync")?;
    let connection = crate::store::open_existing(&synced.database_path).check("open the index")?;
    let mut statement = connection
        .prepare(
            "SELECT json_extract(e.payload, '$.text') FROM evidence e \
             WHERE e.kind = 'Comment' ORDER BY e.id",
        )
        .check("prepare")?;
    let by_id: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .check("query")?
        .collect::<rusqlite::Result<_>>()
        .check("rows")?;

    let response = run_on(&manager, request(project.path(), Some("comment_markers")))?;

    let details: Vec<&str> = full_rows(&response)?
        .iter()
        .map(|row| row.detail.as_str())
        .collect();
    ensure_eq(&details.len(), &3, "details.len() == 3")?;
    ensure(
        full_rows(&response)?.iter().all(|row| row.line == 1),
        "full_rows(&response)?.iter().all(|row| row.line == 1)",
    )?;
    ensure_eq(
        &details,
        &(by_id.iter().map(String::as_str).collect::<Vec<_>>()),
        "details == by_id.iter().map(String::as_str).collect::<Vec<_>>()",
    )?;
    let sql = crate::tools::leads_mechanics::MECHANICS
        .iter()
        .find(|mechanic| mechanic.id == "comment_markers")
        .check("comment_markers is built in")?
        .order_expr();
    ensure_eq(
        &sql,
        &"f.path, e.start_line, e.id",
        "sql == \"f.path, e.start_line, e.id\"",
    )?;
    Ok(())
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
fn paged_leads_custom_scan_stopped_by_deadline_is_partial_on_every_page() -> TestResult {
    let project = marker_fixture(3, 4)?;
    fs::write(project.path().join("app.py"), "eval(\"1\")\n").check("write python")?;
    let manager = crate::sync::SourceSyncManager::default();
    manager
        .session_mechanics()
        .with_entries(|entries| entries.push(eval_entry()))
        .check("register the session mechanic")?;
    let limits = ResourceLimits {
        max_structure_execution_time: std::time::Duration::ZERO,
        ..ResourceLimits::default()
    };
    let catalog = mechanic_catalog::load(manager.session_mechanics(), &limits)
        .check("load the mechanic catalog")?;
    let mut whole_surface = compact_request(project.path(), None);
    whole_surface.limit = Some(5);

    let first = leads_with_catalog(
        &whole_surface,
        &limits,
        &catalog,
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("a stopped scan is a partial result")?;

    let expected = Some(crate::tools::page::Completeness::Partial {
        limited_by: PageLimit::TimeBudget,
        resume_after_file: None,
        files_scanned: 0,
        files_total: 1,
    });
    ensure_eq(
        &first.page.completeness,
        &expected,
        "first.page.completeness == expected",
    )?;
    let rest = rest_of_limited(&manager, &first, Some(5))?;
    ensure(!rest.is_empty(), "!rest.is_empty()")?;
    for page in &rest {
        ensure_eq(
            &page.page.completeness,
            &expected,
            "page.page.completeness == expected",
        )?;
    }
    Ok(())
}

#[test]
fn paged_leads_compact_pages_by_cursor() -> TestResult {
    let project = marker_fixture(3, 4)?; // 12 markers across 3 files
    let manager = crate::sync::SourceSyncManager::default();
    let mut whole = compact_request(project.path(), None);
    whole.page_bytes = Some(1_000_000);
    let whole = run_on(&manager, whole)?;
    let mut first = compact_request(project.path(), None);
    first.limit = Some(5);
    let first = run_on(&manager, first)?;

    let rest = rest_of_limited(&manager, &first, Some(5))?;

    let key = |row: &CompactLeadRow| (row.mechanic.clone(), row.file.clone(), row.line);
    let mut union: Vec<_> = compact_rows(&first)?.iter().map(key).collect();
    for page in &rest {
        union.extend(compact_rows(page)?.iter().map(key));
    }
    let expected: Vec<_> = compact_rows(&whole)?.iter().map(key).collect();
    ensure_eq(&union, &expected, "union == expected")?;
    ensure_eq(&rest.len(), &2, "rest.len() == 2")?;
    let covered: Vec<i64> = std::iter::once(&first)
        .chain(&rest)
        .map(|page| page.files_covered)
        .collect();
    ensure(
        covered
            .windows(2)
            .all(|pair| matches!(pair, [earlier, later] if earlier <= later)),
        "covered.windows(2).all(|pair| pair[0] <= pair[1])",
    )?;
    ensure_eq(
        &covered.last().copied(),
        &Some(first.files_total),
        "covered.last().copied() == Some(first.files_total)",
    )?;
    ensure(
        rest.last().check("a last page")?.page.complete,
        "rest.last().check(\"a last page\")?.page.complete",
    )?;
    Ok(())
}

/// A continuation exactly as a caller sends it when it repeats nothing but the
/// cursor: no `page_bytes`, no `limit`.
fn bare_continuation(page: &LeadsResponse) -> TestResult<LeadsRequest> {
    Ok(LeadsRequest {
        page_bytes: None,
        ..continuation(page)?
    })
}

fn run_with_defaults(
    manager: &crate::sync::SourceSyncManager,
    request: LeadsRequest,
    default_page_bytes: usize,
) -> TestResult<LeadsResponse> {
    let limits = ResourceLimits {
        default_page_bytes,
        ..ResourceLimits::default()
    };
    let Json(response) = leads_with_limits(
        &Parameters(request),
        &limits,
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("leads succeeds")?;
    Ok(response)
}

#[test]
fn paged_leads_continuation_keeps_the_page_budget_of_page_one() -> TestResult {
    let project = marker_fixture(4, 6)?; // 24 rows
    let manager = crate::sync::SourceSyncManager::default();
    let first = run_with_defaults(
        &manager,
        request(project.path(), Some("comment_markers")),
        SMALL_PAGE,
    )?;
    ensure_eq(
        &first.page.page_bytes,
        &SMALL_PAGE,
        "first.page.page_bytes == SMALL_PAGE",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("a small default forces paging")?;

    let mut seen = places(&first)?;
    let mut last = first;
    while last.page.cursor.is_some() {
        // The process default is 64 KiB: only the stored budget can cut here.
        last = run_with_defaults(&manager, bare_continuation(&last)?, 1_000_000)?;
        ensure_eq(
            &last.page.page_bytes,
            &SMALL_PAGE,
            "last.page.page_bytes == SMALL_PAGE",
        )?;
        ensure(
            last.page.bytes_used <= SMALL_PAGE || last.page.returned == 1,
            "last.page.bytes_used <= SMALL_PAGE || last.page.returned == 1",
        )?;
        seen.extend(places(&last)?);
    }

    ensure_eq(&seen.len(), &24, "seen.len() == 24")?;
    Ok(())
}

#[test]
fn paged_leads_continuation_keeps_the_limit_of_page_one() -> TestResult {
    let project = marker_fixture(4, 6)?; // 24 rows
    let manager = crate::sync::SourceSyncManager::default();
    let mut asked = request(project.path(), Some("comment_markers"));
    asked.limit = Some(5);
    let first = run_on(&manager, asked)?;
    ensure_eq(&first.page.returned, &5, "first.page.returned == 5")?;

    let second = run_on(&manager, bare_continuation(&first)?)?;
    let mut own = bare_continuation(&first)?;
    own.limit = Some(2);
    let smaller = run_on(&manager, own)?;

    ensure_eq(&second.page.returned, &5, "second.page.returned == 5")?;
    ensure_eq(
        &second.page.limited_by,
        &PageLimit::Limit,
        "second.page.limited_by == PageLimit::Limit",
    )?;
    ensure_eq(&smaller.page.returned, &2, "smaller.page.returned == 2")?;
    Ok(())
}

/// A continuation's explicit `page_bytes` is clamped to the ceiling page one
/// ran under (stored with the result), not to the limits of the call that
/// serves it, and the cut reports `ceiling`.
#[test]
fn paged_leads_continuation_clamps_page_bytes_to_the_ceiling_of_page_one() -> TestResult {
    const CEILING: usize = 300;
    let project = marker_fixture(4, 6)?; // 24 rows
    let manager = crate::sync::SourceSyncManager::default();
    let Json(first) = leads_with_limits(
        &Parameters(request(project.path(), Some("comment_markers"))),
        &ResourceLimits {
            max_query_response_bytes: CEILING,
            ..ResourceLimits::default()
        },
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("page one under a small ceiling")?;
    ensure_eq(
        &first.page.page_bytes,
        &CEILING,
        "first.page.page_bytes == CEILING",
    )?;
    let mut asked = continuation(&first)?;
    asked.page_bytes = Some(1_000_000);

    // The serving call's own ceiling is far above the 1 MB asked for.
    let second = run_on(&manager, asked)?;

    ensure_eq(
        &second.page.page_bytes,
        &CEILING,
        "second.page.page_bytes == CEILING",
    )?;
    ensure_eq(
        &second.page.limited_by,
        &PageLimit::Ceiling,
        "second.page.limited_by == PageLimit::Ceiling",
    )?;
    Ok(())
}
