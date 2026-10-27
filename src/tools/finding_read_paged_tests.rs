//! Cursor paging of `finding_read`: the stored result is one immutable answer.
use super::tests::{
    cursor_request, db_path, request, with_stable_session, write_finding,
    write_finding_with_description,
};
use super::*;
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;

const ROWS: usize = 20;

fn read(
    manager: &SourceSyncManager,
    request: FindingReadRequest,
) -> Result<FindingReadResponse, ErrorData> {
    finding_read(&Parameters(request), &NoopProgressSink, manager).map(|Json(response)| response)
}

fn error_code(error: &ErrorData) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(serde_json::Value::as_str)
}

/// A project with `ROWS` findings of 10,000 characters each, so the default
/// page budget forces several pages.
fn project_with_findings(manager: &SourceSyncManager) -> (tempfile::TempDir, String, Vec<i64>) {
    let project = activated_project("lib.rs", b"fn main() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let description = "d".repeat(10_000);
    let ids = (0..ROWS)
        .map(|index| {
            write_finding_with_description(
                &project,
                "lib.rs",
                &format!("finding {index}"),
                &description,
                manager,
            )
            .id
        })
        .collect();
    (project, path, ids)
}

/// Every page after `first`, followed by cursor only.
fn rest_of(manager: &SourceSyncManager, first: &FindingReadResponse) -> Vec<FindingReadResponse> {
    let mut pages = Vec::new();
    let mut cursor = first.page.cursor.clone();
    while let Some(token) = cursor {
        let page = read(manager, cursor_request(&token)).expect("continuation page");
        cursor = page.page.cursor.clone();
        pages.push(page);
    }
    pages
}

fn ids_of(pages: &[&FindingReadResponse]) -> Vec<i64> {
    pages
        .iter()
        .flat_map(|page| page.findings.iter().map(|entry| entry.id))
        .collect()
}

#[test]
fn paged_finding_read_pages_cover_every_row_once_within_the_byte_budget() {
    with_stable_session(|| {
        let manager = SourceSyncManager::default();
        let (_project, path, written) = project_with_findings(&manager);

        let first = read(&manager, request(&path, None)).expect("page one");
        let rest = rest_of(&manager, &first);

        let pages: Vec<&FindingReadResponse> = std::iter::once(&first).chain(&rest).collect();
        assert!(pages.len() >= 3, "the fixture must need several pages");
        assert_eq!(ids_of(&pages), written, "rows in id order, once each");
        for (index, page) in pages.iter().enumerate() {
            assert!(page.page.bytes_used <= page.page.page_bytes, "page {index}");
            assert_eq!(page.page.total, Some(ROWS as u64));
            assert_eq!(page.page.returned, page.findings.len());
            assert_eq!(page.page.revision_id, first.page.revision_id);
            assert_eq!(page.page.next_offset, None);
            let last = index + 1 == pages.len();
            assert_eq!(page.page.complete, last);
            assert_eq!(page.page.cursor.is_none(), last);
            let expected = if last {
                PageLimit::None
            } else {
                PageLimit::PageBytes
            };
            assert_eq!(page.page.limited_by, expected);
        }
    });
}

#[test]
fn paged_finding_read_pages_survive_an_edit_between_pages() {
    with_stable_session(|| {
        let manager = SourceSyncManager::default();
        let (project, path, written) = project_with_findings(&manager);
        let first = read(&manager, request(&path, None)).expect("page one");
        let cursor = first.page.cursor.clone().expect("several pages");

        // One matching row vanishes (deleted straight from the database) and
        // one new row appears.
        let vanished = written[written.len() - 1];
        let connection = crate::store::open_read_write(&db_path(&project)).expect("open db");
        connection
            .execute("DELETE FROM findings WHERE id = ?1", [vanished])
            .expect("delete a finding");
        drop(connection);
        let appeared = write_finding(&project, "lib.rs", "recorded between pages", &manager).id;
        let rest = rest_of(&manager, &first);

        let pages: Vec<&FindingReadResponse> = std::iter::once(&first).chain(&rest).collect();
        assert_eq!(ids_of(&pages), written, "the stored answer is unchanged");
        assert!(rest.iter().all(|page| page.page.total == first.page.total));
        assert!(
            rest.iter()
                .all(|page| page.page.revision_id == first.page.revision_id)
        );
        assert_eq!(first.page.cursor.as_deref(), Some(cursor.as_str()));
        let fresh = read(&manager, request(&path, None)).expect("fresh request");
        let fresh_rest = rest_of(&manager, &fresh);
        let fresh_pages: Vec<&FindingReadResponse> =
            std::iter::once(&fresh).chain(&fresh_rest).collect();
        let fresh_ids = ids_of(&fresh_pages);
        assert!(
            !fresh_ids.contains(&vanished),
            "a fresh read lacks the deleted row"
        );
        assert!(
            fresh_ids.contains(&appeared),
            "a fresh read has the new row"
        );
        assert_eq!(fresh.page.total, Some(ROWS as u64));
    });
}

#[test]
fn paged_finding_read_new_finding_after_page_one_is_not_in_the_stored_result_but_in_a_fresh_request()
 {
    with_stable_session(|| {
        let manager = SourceSyncManager::default();
        let (project, path, written) = project_with_findings(&manager);
        let first = read(&manager, request(&path, None)).expect("page one");
        let appeared = write_finding(&project, "lib.rs", "late", &manager).id;

        let rest = rest_of(&manager, &first);
        let stored: Vec<&FindingReadResponse> = std::iter::once(&first).chain(&rest).collect();
        let fresh = read(&manager, request(&path, None)).expect("fresh request");

        assert_eq!(ids_of(&stored), written);
        assert_eq!(fresh.page.total, Some(ROWS as u64 + 1));
        let fresh_rest = rest_of(&manager, &fresh);
        let fresh_all: Vec<&FindingReadResponse> =
            std::iter::once(&fresh).chain(&fresh_rest).collect();
        assert_eq!(ids_of(&fresh_all).last(), Some(&appeared));
    });
}

#[test]
fn paged_finding_read_same_cursor_twice_returns_identical_pages() {
    with_stable_session(|| {
        let manager = SourceSyncManager::default();
        let (_project, path, _) = project_with_findings(&manager);
        let token = read(&manager, request(&path, None))
            .expect("page one")
            .page
            .cursor
            .expect("several pages");

        let one = read(&manager, cursor_request(&token)).expect("first read");
        let two = read(&manager, cursor_request(&token)).expect("second read");

        assert_eq!(ids_of(&[&one]), ids_of(&[&two]));
        assert_eq!(one.page, two.page);
        assert_eq!(
            serde_json::to_value(&one).expect("serializes"),
            serde_json::to_value(&two).expect("serializes")
        );
    });
}

#[test]
fn paged_finding_read_final_page_can_be_retried() {
    with_stable_session(|| {
        let manager = SourceSyncManager::default();
        let (_project, path, _) = project_with_findings(&manager);
        let first = read(&manager, request(&path, None)).expect("page one");
        let mut token = first.page.cursor.clone().expect("several pages");
        let mut final_token = token.clone();
        let last = loop {
            let page = read(&manager, cursor_request(&token)).expect("page");
            match page.page.cursor.clone() {
                Some(next) => {
                    final_token = token;
                    token = next;
                }
                None => break page,
            }
        };
        // `final_token` produced the page before `last`; `token` produced `last`.

        let again = read(&manager, cursor_request(&token)).expect("retry of the final page");

        assert_ne!(final_token, token);
        assert!(again.page.complete);
        assert_eq!(again.page.cursor, None);
        assert_eq!(again.page.limited_by, PageLimit::None);
        assert_eq!(again.page, last.page);
        assert_eq!(ids_of(&[&again]), ids_of(&[&last]));
    });
}

#[test]
fn paged_finding_read_refuses_offset_and_expected_revision_naming_cursor() {
    let manager = SourceSyncManager::default();
    for extra in [
        serde_json::json!({ "offset": 1 }),
        serde_json::json!({ "expected_revision": "r1" }),
    ] {
        let mut body = serde_json::json!({ "path": "/nowhere" });
        body.as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        let parsed: FindingReadRequest = serde_json::from_value(body).expect("still parses");

        let error = read(&manager, parsed).expect_err("removed parameters are refused");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(error_code(&error), Some("paging_parameter_removed"));
        assert!(error.message.contains("cursor"), "{}", error.message);
    }
}

#[test]
fn paged_finding_read_without_cursor_or_path_is_refused() {
    let manager = SourceSyncManager::default();
    let neither = FindingReadRequest {
        path: None,
        ..request("", None)
    };

    let error = read(&manager, neither).expect_err("path is required without a cursor");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "path is required when no cursor is given");
}

#[test]
fn paged_finding_read_unknown_cursor_is_refused_with_unknown_cursor() {
    let manager = SourceSyncManager::default();

    let error = read(&manager, cursor_request("v1.not-a-real-token"))
        .expect_err("a forged cursor is refused");

    assert_eq!(error_code(&error), Some("unknown_cursor"));
}

#[test]
fn paged_finding_read_cursor_ignores_path_and_file() {
    with_stable_session(|| {
        let manager = SourceSyncManager::default();
        let (_project, path, _) = project_with_findings(&manager);
        let token = read(&manager, request(&path, None))
            .expect("page one")
            .page
            .cursor
            .expect("several pages");
        let plain = read(&manager, cursor_request(&token)).expect("cursor only");

        let shaped = read(
            &manager,
            FindingReadRequest {
                path: Some("/not/a/project".to_owned()),
                file: Some("other.rs".to_owned()),
                ..cursor_request(&token)
            },
        )
        .expect("path and file are ignored with a cursor");

        assert_eq!(ids_of(&[&plain]), ids_of(&[&shaped]));
        assert_eq!(plain.page, shaped.page);
    });
}
