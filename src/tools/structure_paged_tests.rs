//! Tests for `structure` paging on the result store: a cursor pages one stored
//! answer, a window-limited scan is a partial result, and resuming it is a new
//! request.

use super::limit_tests::{
    many_functions, page_request as shaped_request, run_with, run_with_manager,
};
use super::*;
use crate::sync::SourceSyncManager;
use crate::tools::PageLimit;
use crate::tools::page::Completeness;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};

const QUERY: &str = "(function_item name: (identifier) @name)";
/// About four matches per page: each serialized match is roughly 200 bytes.
const SMALL_PAGE: usize = 900;

fn functions(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("pub fn {prefix}{index:02}() {{}}\n"))
        .collect::<Vec<_>>()
        .concat()
}

fn two_file_project() -> TestResult<tempfile::TempDir> {
    let project = activated_project("a.rs", functions("a", 20).as_bytes())?;
    std::fs::write(project.path().join("b.rs"), functions("b", 20)).check("write b.rs")?;
    Ok(project)
}

fn request(
    project: &tempfile::TempDir,
    cursor: Option<&str>,
    page_bytes: Option<usize>,
) -> Parameters<StructureRequest> {
    Parameters(StructureRequest {
        path: project.path().to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        query: QUERY.to_owned(),
        cursor: cursor.map(str::to_owned),
        page_bytes,
        ..Default::default()
    })
}

fn call(
    request: &Parameters<StructureRequest>,
    window_bytes: usize,
    manager: &SourceSyncManager,
) -> Result<StructureResponse, ErrorData> {
    structure_with_limits(
        request,
        &ResourceLimits::default(),
        window_bytes,
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
}

fn page_of(
    project: &tempfile::TempDir,
    cursor: Option<&str>,
    page_bytes: Option<usize>,
    manager: &SourceSyncManager,
) -> TestResult<StructureResponse> {
    call(
        &request(project, cursor, page_bytes),
        MAX_MULTI_FILE_TOTAL_BYTES,
        manager,
    )
    .check("structure call succeeds")
}

fn names(response: &StructureResponse) -> Vec<String> {
    response.matches.iter().map(|m| m.text.clone()).collect()
}

/// Every page of the result `first` starts, by cursor, as `(cursor used, page)`.
fn rest_of(
    project: &tempfile::TempDir,
    first: &StructureResponse,
    manager: &SourceSyncManager,
) -> TestResult<Vec<(String, StructureResponse)>> {
    let mut pages = Vec::new();
    let mut cursor = first.page.cursor.clone();
    while let Some(token) = cursor {
        let page = page_of(project, Some(&token), Some(SMALL_PAGE), manager)?;
        cursor = page.page.cursor.clone();
        pages.push((token, page));
    }
    Ok(pages)
}

#[test]
fn paged_structure_pages_survive_an_edit_between_pages() -> TestResult {
    let project = two_file_project()?;
    let manager = SourceSyncManager::default();
    let full = page_of(&project, None, None, &manager)?;
    ensure_eq(&full.matches.len(), &40, "full.matches.len() == 40")?;
    let first = page_of(&project, None, Some(SMALL_PAGE), &manager)?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;

    let edited = functions("a", 20).replace("pub fn a05() {}\n", "pub fn zz_new() {}\n");
    std::fs::write(project.path().join("a.rs"), edited).check("edit a.rs")?;
    let rest = rest_of(&project, &first, &manager)?;

    ensure(rest.len() >= 2, "page one plus two more pages at least")?;
    let mut union = names(&first);
    for (_, page) in &rest {
        union.extend(names(page));
        ensure_eq(
            &page.page.revision_id,
            &first.page.revision_id,
            "page.page.revision_id == first.page.revision_id",
        )?;
        ensure_eq(
            &page.page.total,
            &first.page.total,
            "page.page.total == first.page.total",
        )?;
    }
    ensure_eq(&union, &names(&full), "union == names(&full)")?;
    ensure(
        union.contains(&"a05".to_owned()) && !union.contains(&"zz_new".to_owned()),
        "union.contains(&\"a05\".to_owned()) && !union.contains(&\"zz_new\".to_owned())",
    )?;
    let fresh = page_of(&project, None, None, &manager)?;
    ensure(
        names(&fresh).contains(&"zz_new".to_owned()),
        "names(&fresh).contains(&\"zz_new\".to_owned())",
    )?;
    ensure(
        !names(&fresh).contains(&"a05".to_owned()),
        "!names(&fresh).contains(&\"a05\".to_owned())",
    )?;
    ensure_ne(
        &fresh.page.revision_id,
        &first.page.revision_id,
        "fresh.page.revision_id != first.page.revision_id",
    )?;
    Ok(())
}

#[test]
fn paged_structure_same_cursor_twice_returns_identical_pages() -> TestResult {
    let project = two_file_project()?;
    let manager = SourceSyncManager::default();
    let first = page_of(&project, None, Some(SMALL_PAGE), &manager)?;
    let token = first.page.cursor.check("a second page exists")?;

    let one = page_of(&project, Some(&token), Some(SMALL_PAGE), &manager)?;
    let two = page_of(&project, Some(&token), Some(SMALL_PAGE), &manager)?;

    ensure_eq(
        &serde_json::to_value(&one).check("serializes")?,
        &serde_json::to_value(&two).check("serializes")?,
        "serde_json::to_value(&one).check(\"serializes\")? == serde_json::to_value(&two).check(\"serializes\")?",
    )?;
    ensure(!one.matches.is_empty(), "!one.matches.is_empty()")?;
    Ok(())
}

#[test]
fn paged_structure_final_page_can_be_retried() -> TestResult {
    let project = two_file_project()?;
    let manager = SourceSyncManager::default();
    let first = page_of(&project, None, Some(SMALL_PAGE), &manager)?;
    let rest = rest_of(&project, &first, &manager)?;
    let (last_token, last) = rest.last().check("several pages")?;

    let again = page_of(&project, Some(last_token), Some(SMALL_PAGE), &manager)?;

    ensure_eq(
        &serde_json::to_value(&again).check("serializes")?,
        &serde_json::to_value(last).check("serializes")?,
        "serde_json::to_value(&again).check(\"serializes\")? == serde_json::to_value(last).check(\"serializes\")?",
    )?;
    ensure(
        again.page.cursor.is_none() && again.page.complete,
        "again.page.cursor.is_none() && again.page.complete",
    )?;
    ensure_eq(
        &again.page.limited_by,
        &PageLimit::None,
        "again.page.limited_by == PageLimit::None",
    )?;
    Ok(())
}

#[test]
fn paged_structure_refuses_offset_and_expected_revision_naming_cursor() -> TestResult {
    let project = two_file_project()?;
    let path = project.path().to_string_lossy().into_owned();
    for field in [
        serde_json::json!({ "offset": 3 }),
        serde_json::json!({ "expected_revision": "r1" }),
    ] {
        let mut value = serde_json::json!({ "path": path, "language": "rust", "query": QUERY });
        value
            .as_object_mut()
            .check("an object")?
            .extend(field.as_object().check("an object")?.clone());
        let parsed: StructureRequest = serde_json::from_value(value).check("parses")?;

        let error = call(
            &Parameters(parsed),
            MAX_MULTI_FILE_TOTAL_BYTES,
            &SourceSyncManager::default(),
        )
        .check_err("a removed paging parameter is refused")?;

        let data = error.data.check("a payload")?;
        ensure_eq(
            &data.get("code").check("data[\"code\"]")?,
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

/// Three 18-byte files; an 18-byte window holds exactly one of them.
fn three_small_files() -> TestResult<tempfile::TempDir> {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n")?;
    for (name, body) in [
        ("b.rs", "pub fn beta() {}\n"),
        ("c.rs", "pub fn gamma() {}\n"),
    ] {
        std::fs::write(project.path().join(name), body).check("write fixture")?;
    }
    Ok(project)
}

#[test]
fn paged_structure_window_limited_result_is_partial_and_resumes_as_a_new_request() -> TestResult {
    let project = three_small_files()?;
    let manager = SourceSyncManager::default();

    let first = call(&request(&project, None, None), 18, &manager).check("first window")?;

    ensure_eq(&names(&first), &["alpha"], "names(&first) == [\"alpha\"]")?;
    ensure(
        !first.page.complete && first.page.cursor.is_none(),
        "!first.page.complete && first.page.cursor.is_none()",
    )?;
    ensure_eq(
        &first.page.limited_by,
        &PageLimit::ScanWindow,
        "first.page.limited_by == PageLimit::ScanWindow",
    )?;
    ensure_eq(
        &first.page.completeness,
        &Some(Completeness::Partial {
            limited_by: PageLimit::ScanWindow,
            resume_after_file: Some("a.rs".to_owned()),
            files_scanned: 1,
            files_total: 3,
        }),
        "first.page.completeness == Some(Completeness::Partial { limited_by: PageLimit::ScanWindow, resume_after_file: Some(\"a.rs\".to_owned()), files_scanned: 1, files_total: 3, })",
    )?;
    ensure_eq(
        &first.next_start_after_file.as_deref(),
        &Some("a.rs"),
        "first.next_start_after_file.as_deref() == Some(\"a.rs\")",
    )?;
    let mut resume = request(&project, None, None);
    resume.0.start_after_file = first.next_start_after_file.clone();
    let second = call(&resume, 18, &manager).check("resumed window")?;
    ensure_eq(&names(&second), &["beta"], "names(&second) == [\"beta\"]")?;
    ensure_eq(
        &second.next_start_after_file.as_deref(),
        &Some("b.rs"),
        "second.next_start_after_file.as_deref() == Some(\"b.rs\")",
    )?;
    resume.0.start_after_file = second.next_start_after_file.clone();
    let third = call(&resume, 18, &manager).check("last window")?;
    ensure_eq(&names(&third), &["gamma"], "names(&third) == [\"gamma\"]")?;
    ensure_eq(
        &third.page.completeness,
        &Some(Completeness::Complete),
        "third.page.completeness == Some(Completeness::Complete)",
    )?;
    ensure(third.page.complete, "third.page.complete")?;
    Ok(())
}

#[test]
fn paged_structure_cursor_never_triggers_a_new_scan_window() -> TestResult {
    let project = activated_project("a.rs", functions("a", 30).as_bytes())?;
    std::fs::write(project.path().join("b.rs"), functions("b", 5)).check("write b.rs")?;
    let window = functions("a", 30).len();
    let manager = SourceSyncManager::default();
    let first =
        call(&request(&project, None, Some(SMALL_PAGE)), window, &manager).check("first window")?;
    ensure_eq(
        &first.next_start_after_file.as_deref(),
        &Some("a.rs"),
        "first.next_start_after_file.as_deref() == Some(\"a.rs\")",
    )?;
    std::fs::write(project.path().join("c.rs"), functions("c", 5)).check("write c.rs")?;

    let rest = rest_of(&project, &first, &manager)?;

    ensure(!rest.is_empty(), "!rest.is_empty()")?;
    let mut union = names(&first);
    for (_, page) in &rest {
        union.extend(names(page));
        ensure_eq(
            &page.next_start_after_file,
            &first.next_start_after_file,
            "page.next_start_after_file == first.next_start_after_file",
        )?;
        ensure_eq(
            &page.files_not_scanned,
            &first.files_not_scanned,
            "page.files_not_scanned == first.files_not_scanned",
        )?;
        ensure_eq(
            &page.page.completeness,
            &first.page.completeness,
            "page.page.completeness == first.page.completeness",
        )?;
        ensure_eq(
            &page.page.revision_id,
            &first.page.revision_id,
            "page.page.revision_id == first.page.revision_id",
        )?;
    }
    ensure_eq(&union.len(), &30, "only a.rs: no later window, no new file")?;
    ensure(
        union.iter().all(|name| name.starts_with('a')),
        "union.iter().all(|name| name.starts_with('a'))",
    )?;
    Ok(())
}

#[test]
fn paged_structure_start_after_file_unknown_path_scans_the_files_sorting_after_it() -> TestResult {
    let project = three_small_files()?;
    let mut shaped = request(&project, None, None);
    shaped.0.start_after_file = Some("aa-not-a-file.rs".to_owned());

    let response = call(
        &shaped,
        MAX_MULTI_FILE_TOTAL_BYTES,
        &SourceSyncManager::default(),
    )
    .check("an unknown start path is a plain lower bound, as before")?;

    ensure_eq(
        &names(&response),
        &["beta", "gamma"],
        "names(&response) == [\"beta\", \"gamma\"]",
    )?;
    Ok(())
}

#[test]
fn a_page_cut_by_bytes_continues_through_the_cursor_to_every_match() -> TestResult {
    let project = activated_project("lib.rs", many_functions(30).as_bytes())?;
    let limits = ResourceLimits::default();
    let manager = crate::sync::SourceSyncManager::default();
    let mut names = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;

    loop {
        let response = run_with_manager(
            &shaped_request(&project, cursor.as_deref(), Some(900)),
            &limits,
            usize::MAX,
            &manager,
        )?;
        pages += 1;
        names.extend(response.matches.iter().map(|m| m.text.clone()));
        ensure_eq(
            &!response.page.complete,
            &response.page.cursor.is_some(),
            "!response.page.complete == response.page.cursor.is_some()",
        )?;
        ensure_eq(
            &response.page.returned,
            &response.matches.len(),
            "response.page.returned == response.matches.len()",
        )?;
        ensure_eq(&response.page.total, &None, "structure never counts ahead")?;
        cursor = response.page.cursor;
        if cursor.is_none() {
            break;
        }
    }

    let expected: Vec<String> = (0..30).map(|index| format!("f{index:02}")).collect();
    ensure_eq(&names, &expected, "names == expected")?;
    ensure(pages > 1, "a 900-byte page must not hold 30 matches")?;
    Ok(())
}

#[test]
fn a_page_bytes_override_changes_the_page_and_is_echoed() -> TestResult {
    let project = activated_project("lib.rs", many_functions(30).as_bytes())?;
    let limits = ResourceLimits::default();

    let small = run_with(
        &shaped_request(&project, None, Some(900)),
        &limits,
        usize::MAX,
    )?;
    let default = run_with(&shaped_request(&project, None, None), &limits, usize::MAX)?;

    ensure_eq(&small.page.page_bytes, &900, "small.page.page_bytes == 900")?;
    ensure_eq(
        &default.page.page_bytes,
        &limits.default_page_bytes,
        "default.page.page_bytes == limits.default_page_bytes",
    )?;
    ensure(
        small.matches.len() < default.matches.len(),
        "small.matches.len() < default.matches.len()",
    )?;
    ensure_eq(&default.matches.len(), &30, "default.matches.len() == 30")?;
    ensure(
        default.page.complete && default.page.cursor.is_none(),
        "default.page.complete && default.page.cursor.is_none()",
    )?;
    ensure_eq(
        &default.page.limited_by,
        &PageLimit::None,
        "default.page.limited_by == PageLimit::None",
    )?;
    ensure_eq(
        &small.page.limited_by,
        &PageLimit::PageBytes,
        "small.page.limited_by == PageLimit::PageBytes",
    )?;
    ensure(small.page.bytes_used <= 900, "small.page.bytes_used <= 900")?;
    Ok(())
}

#[test]
fn a_page_bytes_above_the_ceiling_is_clamped_and_a_cut_reports_the_ceiling() -> TestResult {
    let project = activated_project("lib.rs", many_functions(100).as_bytes())?;
    let limits = ResourceLimits {
        max_query_response_bytes: 4_096,
        ..ResourceLimits::default()
    };

    let response = run_with(
        &shaped_request(&project, None, Some(10_000_000)),
        &limits,
        usize::MAX,
    )?;

    ensure_eq(
        &response.page.page_bytes,
        &4_096,
        "response.page.page_bytes == 4_096",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::Ceiling,
        "response.page.limited_by == PageLimit::Ceiling",
    )?;
    ensure(!response.page.complete, "!response.page.complete")?;
    Ok(())
}

#[test]
fn a_match_count_exactly_at_the_cap_is_not_reported_as_cut() -> TestResult {
    let project = activated_project("lib.rs", many_functions(4).as_bytes())?;
    let limits = ResourceLimits {
        max_structure_matches: 4,
        ..ResourceLimits::default()
    };

    let response = run_with(&shaped_request(&project, None, None), &limits, usize::MAX)?;

    ensure_eq(&response.matches.len(), &4, "response.matches.len() == 4")?;
    ensure(
        response.page.complete && response.page.cursor.is_none(),
        "response.page.complete && response.page.cursor.is_none()",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::None,
        "response.page.limited_by == PageLimit::None",
    )?;
    Ok(())
}

#[test]
fn paged_structure_continuation_keeps_the_page_budget_of_page_one() -> TestResult {
    let project = two_file_project()?;
    let manager = SourceSyncManager::default();
    let small_default = ResourceLimits {
        default_page_bytes: SMALL_PAGE,
        ..ResourceLimits::default()
    };
    let first = structure_with_limits(
        &request(&project, None, None),
        &small_default,
        MAX_MULTI_FILE_TOTAL_BYTES,
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .map(|Json(response)| response)
    .check("page one succeeds")?;
    let token = first.page.cursor.clone().check("a second page exists")?;

    // The continuation names no page_bytes and runs under the larger default.
    let second = page_of(&project, Some(&token), None, &manager)?;

    ensure_eq(
        &first.page.page_bytes,
        &SMALL_PAGE,
        "first.page.page_bytes == SMALL_PAGE",
    )?;
    ensure_eq(
        &second.page.page_bytes,
        &SMALL_PAGE,
        "second.page.page_bytes == SMALL_PAGE",
    )?;
    ensure(
        second.page.bytes_used <= SMALL_PAGE,
        "second.page.bytes_used <= SMALL_PAGE",
    )?;
    ensure_eq(
        &second.page.limited_by,
        &PageLimit::PageBytes,
        "second.page.limited_by == PageLimit::PageBytes",
    )?;
    second.page.cursor.as_ref().check("still several pages")?;
    Ok(())
}
