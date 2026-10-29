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

const QUERY: &str = "(function_item name: (identifier) @name)";
/// About four matches per page: each serialized match is roughly 200 bytes.
const SMALL_PAGE: usize = 900;

fn functions(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("pub fn {prefix}{index:02}() {{}}\n"))
        .collect()
}

fn two_file_project() -> tempfile::TempDir {
    let project = activated_project("a.rs", functions("a", 20).as_bytes());
    std::fs::write(project.path().join("b.rs"), functions("b", 20)).expect("write b.rs");
    project
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
) -> StructureResponse {
    call(
        &request(project, cursor, page_bytes),
        MAX_MULTI_FILE_TOTAL_BYTES,
        manager,
    )
    .expect("structure call succeeds")
}

fn names(response: &StructureResponse) -> Vec<String> {
    response.matches.iter().map(|m| m.text.clone()).collect()
}

/// Every page of the result `first` starts, by cursor, as `(cursor used, page)`.
fn rest_of(
    project: &tempfile::TempDir,
    first: &StructureResponse,
    manager: &SourceSyncManager,
) -> Vec<(String, StructureResponse)> {
    let mut pages = Vec::new();
    let mut cursor = first.page.cursor.clone();
    while let Some(token) = cursor {
        let page = page_of(project, Some(&token), Some(SMALL_PAGE), manager);
        cursor = page.page.cursor.clone();
        pages.push((token, page));
    }
    pages
}

#[test]
fn paged_structure_pages_survive_an_edit_between_pages() {
    let project = two_file_project();
    let manager = SourceSyncManager::default();
    let full = page_of(&project, None, None, &manager);
    assert_eq!(full.matches.len(), 40);
    let first = page_of(&project, None, Some(SMALL_PAGE), &manager);
    assert!(first.page.cursor.is_some());

    let edited = functions("a", 20).replace("pub fn a05() {}\n", "pub fn zz_new() {}\n");
    std::fs::write(project.path().join("a.rs"), edited).expect("edit a.rs");
    let rest = rest_of(&project, &first, &manager);

    assert!(rest.len() >= 2, "page one plus two more pages at least");
    let mut union = names(&first);
    for (_, page) in &rest {
        union.extend(names(page));
        assert_eq!(page.page.revision_id, first.page.revision_id);
        assert_eq!(page.page.total, first.page.total);
    }
    assert_eq!(union, names(&full));
    assert!(union.contains(&"a05".to_owned()) && !union.contains(&"zz_new".to_owned()));
    let fresh = page_of(&project, None, None, &manager);
    assert!(names(&fresh).contains(&"zz_new".to_owned()));
    assert!(!names(&fresh).contains(&"a05".to_owned()));
    assert_ne!(fresh.page.revision_id, first.page.revision_id);
}

#[test]
fn paged_structure_same_cursor_twice_returns_identical_pages() {
    let project = two_file_project();
    let manager = SourceSyncManager::default();
    let first = page_of(&project, None, Some(SMALL_PAGE), &manager);
    let token = first.page.cursor.expect("a second page exists");

    let one = page_of(&project, Some(&token), Some(SMALL_PAGE), &manager);
    let two = page_of(&project, Some(&token), Some(SMALL_PAGE), &manager);

    assert_eq!(
        serde_json::to_value(&one).expect("serializes"),
        serde_json::to_value(&two).expect("serializes")
    );
    assert!(!one.matches.is_empty());
}

#[test]
fn paged_structure_final_page_can_be_retried() {
    let project = two_file_project();
    let manager = SourceSyncManager::default();
    let first = page_of(&project, None, Some(SMALL_PAGE), &manager);
    let rest = rest_of(&project, &first, &manager);
    let (last_token, last) = rest.last().expect("several pages");

    let again = page_of(&project, Some(last_token), Some(SMALL_PAGE), &manager);

    assert_eq!(
        serde_json::to_value(&again).expect("serializes"),
        serde_json::to_value(last).expect("serializes")
    );
    assert!(again.page.cursor.is_none() && again.page.complete);
    assert_eq!(again.page.limited_by, PageLimit::None);
}

#[test]
fn paged_structure_refuses_offset_and_expected_revision_naming_cursor() {
    let project = two_file_project();
    let path = project.path().to_string_lossy().into_owned();
    for field in [
        serde_json::json!({ "offset": 3 }),
        serde_json::json!({ "expected_revision": "r1" }),
    ] {
        let mut value = serde_json::json!({ "path": path, "language": "rust", "query": QUERY });
        value
            .as_object_mut()
            .expect("an object")
            .extend(field.as_object().expect("an object").clone());
        let parsed: StructureRequest = serde_json::from_value(value).expect("parses");

        let error = call(
            &Parameters(parsed),
            MAX_MULTI_FILE_TOTAL_BYTES,
            &SourceSyncManager::default(),
        )
        .expect_err("a removed paging parameter is refused");

        let data = error.data.expect("a payload");
        assert_eq!(data["code"], "paging_parameter_removed");
        assert!(data["remedy"].as_str().expect("remedy").contains("cursor"));
    }
}

/// Three 18-byte files; an 18-byte window holds exactly one of them.
fn three_small_files() -> tempfile::TempDir {
    let project = activated_project("a.rs", b"pub fn alpha() {}\n");
    for (name, body) in [
        ("b.rs", "pub fn beta() {}\n"),
        ("c.rs", "pub fn gamma() {}\n"),
    ] {
        std::fs::write(project.path().join(name), body).expect("write fixture");
    }
    project
}

#[test]
fn paged_structure_window_limited_result_is_partial_and_resumes_as_a_new_request() {
    let project = three_small_files();
    let manager = SourceSyncManager::default();

    let first = call(&request(&project, None, None), 18, &manager).expect("first window");

    assert_eq!(names(&first), ["alpha"]);
    assert!(!first.page.complete && first.page.cursor.is_none());
    assert_eq!(first.page.limited_by, PageLimit::ScanWindow);
    assert_eq!(
        first.page.completeness,
        Some(Completeness::Partial {
            limited_by: PageLimit::ScanWindow,
            resume_after_file: Some("a.rs".to_owned()),
            files_scanned: 1,
            files_total: 3,
        })
    );
    assert_eq!(first.next_start_after_file.as_deref(), Some("a.rs"));
    let mut resume = request(&project, None, None);
    resume.0.start_after_file = first.next_start_after_file.clone();
    let second = call(&resume, 18, &manager).expect("resumed window");
    assert_eq!(names(&second), ["beta"]);
    assert_eq!(second.next_start_after_file.as_deref(), Some("b.rs"));
    resume.0.start_after_file = second.next_start_after_file.clone();
    let third = call(&resume, 18, &manager).expect("last window");
    assert_eq!(names(&third), ["gamma"]);
    assert_eq!(third.page.completeness, Some(Completeness::Complete));
    assert!(third.page.complete);
}

#[test]
fn paged_structure_cursor_never_triggers_a_new_scan_window() {
    let project = activated_project("a.rs", functions("a", 30).as_bytes());
    std::fs::write(project.path().join("b.rs"), functions("b", 5)).expect("write b.rs");
    let window = functions("a", 30).len();
    let manager = SourceSyncManager::default();
    let first =
        call(&request(&project, None, Some(SMALL_PAGE)), window, &manager).expect("first window");
    assert_eq!(first.next_start_after_file.as_deref(), Some("a.rs"));
    std::fs::write(project.path().join("c.rs"), functions("c", 5)).expect("write c.rs");

    let rest = rest_of(&project, &first, &manager);

    assert!(!rest.is_empty());
    let mut union = names(&first);
    for (_, page) in &rest {
        union.extend(names(page));
        assert_eq!(page.next_start_after_file, first.next_start_after_file);
        assert_eq!(page.files_not_scanned, first.files_not_scanned);
        assert_eq!(page.page.completeness, first.page.completeness);
        assert_eq!(page.page.revision_id, first.page.revision_id);
    }
    assert_eq!(union.len(), 30, "only a.rs: no later window, no new file");
    assert!(union.iter().all(|name| name.starts_with('a')));
}

#[test]
fn paged_structure_start_after_file_unknown_path_scans_the_files_sorting_after_it() {
    let project = three_small_files();
    let mut shaped = request(&project, None, None);
    shaped.0.start_after_file = Some("aa-not-a-file.rs".to_owned());

    let response = call(
        &shaped,
        MAX_MULTI_FILE_TOTAL_BYTES,
        &SourceSyncManager::default(),
    )
    .expect("an unknown start path is a plain lower bound, as before");

    assert_eq!(names(&response), ["beta", "gamma"]);
}

#[test]
fn a_page_cut_by_bytes_continues_through_the_cursor_to_every_match() {
    let project = activated_project("lib.rs", many_functions(30).as_bytes());
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
        );
        pages += 1;
        names.extend(response.matches.iter().map(|m| m.text.clone()));
        assert_eq!(!response.page.complete, response.page.cursor.is_some());
        assert_eq!(response.page.returned, response.matches.len());
        assert_eq!(response.page.total, None, "structure never counts ahead");
        cursor = response.page.cursor;
        if cursor.is_none() {
            break;
        }
    }

    let expected: Vec<String> = (0..30).map(|index| format!("f{index:02}")).collect();
    assert_eq!(names, expected);
    assert!(pages > 1, "a 900-byte page must not hold 30 matches");
}

#[test]
fn a_page_bytes_override_changes_the_page_and_is_echoed() {
    let project = activated_project("lib.rs", many_functions(30).as_bytes());
    let limits = ResourceLimits::default();

    let small = run_with(
        &shaped_request(&project, None, Some(900)),
        &limits,
        usize::MAX,
    );
    let default = run_with(&shaped_request(&project, None, None), &limits, usize::MAX);

    assert_eq!(small.page.page_bytes, 900);
    assert_eq!(default.page.page_bytes, limits.default_page_bytes);
    assert!(small.matches.len() < default.matches.len());
    assert_eq!(default.matches.len(), 30);
    assert!(default.page.complete && default.page.cursor.is_none());
    assert_eq!(default.page.limited_by, PageLimit::None);
    assert_eq!(small.page.limited_by, PageLimit::PageBytes);
    assert!(small.page.bytes_used <= 900);
}

#[test]
fn a_page_bytes_above_the_ceiling_is_clamped_and_a_cut_reports_the_ceiling() {
    let project = activated_project("lib.rs", many_functions(100).as_bytes());
    let limits = ResourceLimits {
        max_query_response_bytes: 4_096,
        ..ResourceLimits::default()
    };

    let response = run_with(
        &shaped_request(&project, None, Some(10_000_000)),
        &limits,
        usize::MAX,
    );

    assert_eq!(response.page.page_bytes, 4_096);
    assert_eq!(response.page.limited_by, PageLimit::Ceiling);
    assert!(!response.page.complete);
}

#[test]
fn a_match_count_exactly_at_the_cap_is_not_reported_as_cut() {
    let project = activated_project("lib.rs", many_functions(4).as_bytes());
    let limits = ResourceLimits {
        max_structure_matches: 4,
        ..ResourceLimits::default()
    };

    let response = run_with(&shaped_request(&project, None, None), &limits, usize::MAX);

    assert_eq!(response.matches.len(), 4);
    assert!(response.page.complete && response.page.cursor.is_none());
    assert_eq!(response.page.limited_by, PageLimit::None);
}

#[test]
fn paged_structure_continuation_keeps_the_page_budget_of_page_one() {
    let project = two_file_project();
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
    .expect("page one succeeds");
    let token = first.page.cursor.clone().expect("a second page exists");

    // The continuation names no page_bytes and runs under the larger default.
    let second = page_of(&project, Some(&token), None, &manager);

    assert_eq!(first.page.page_bytes, SMALL_PAGE);
    assert_eq!(second.page.page_bytes, SMALL_PAGE);
    assert!(second.page.bytes_used <= SMALL_PAGE);
    assert_eq!(second.page.limited_by, PageLimit::PageBytes);
    assert!(second.page.cursor.is_some(), "still several pages");
}
