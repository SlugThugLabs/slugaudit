//! Cursor paging of `finding_read`: the stored result is one immutable answer.
use super::read_tests::{
    cursor_request, findings_db_path, request, write_default_page_bytes, write_finding,
    write_finding_with_description,
};
use super::*;
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::PageLimit;
use crate::tools::test_support::activated_project;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};

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
fn project_with_findings(
    manager: &SourceSyncManager,
) -> TestResult<(tempfile::TempDir, String, Vec<i64>)> {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
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
            .map(|response| response.id)
        })
        .collect::<TestResult<Vec<i64>>>()?;
    Ok((project, path, ids))
}

/// Every page after `first`, followed by cursor only.
fn rest_of(
    manager: &SourceSyncManager,
    first: &FindingReadResponse,
) -> TestResult<Vec<FindingReadResponse>> {
    let mut pages = Vec::new();
    let mut cursor = first.page.cursor.clone();
    while let Some(token) = cursor {
        let page = read(manager, cursor_request(&token)).check("continuation page")?;
        cursor = page.page.cursor.clone();
        pages.push(page);
    }
    Ok(pages)
}

fn ids_of(pages: &[&FindingReadResponse]) -> Vec<i64> {
    pages
        .iter()
        .flat_map(|page| page.findings.iter().map(|entry| entry.id))
        .collect()
}

#[test]
fn paged_finding_read_pages_cover_every_row_once_within_the_byte_budget() -> TestResult {
    let manager = SourceSyncManager::default();
    let (_project, path, written) = project_with_findings(&manager)?;

    let first = read(&manager, request(&path, None)).check("page one")?;
    let rest = rest_of(&manager, &first)?;

    let pages: Vec<&FindingReadResponse> = std::iter::once(&first).chain(&rest).collect();
    ensure(pages.len() >= 3, "pages.len() >= 3")?;
    ensure_eq(&ids_of(&pages), &written, "ids_of(&pages) == written")?;
    for (index, page) in pages.iter().enumerate() {
        ensure(
            page.page.bytes_used <= page.page.page_bytes,
            "page.page.bytes_used <= page.page.page_bytes",
        )?;
        ensure_eq(
            &page.page.total,
            &Some(ROWS as u64),
            "page.page.total == Some(ROWS as u64)",
        )?;
        ensure_eq(
            &page.page.returned,
            &page.findings.len(),
            "page.page.returned == page.findings.len()",
        )?;
        ensure_eq(
            &page.page.revision_id,
            &first.page.revision_id,
            "page.page.revision_id == first.page.revision_id",
        )?;
        let last = index + 1 == pages.len();
        ensure_eq(&page.page.complete, &last, "page.page.complete == last")?;
        ensure_eq(
            &page.page.cursor.is_none(),
            &last,
            "page.page.cursor.is_none() == last",
        )?;
        let expected = if last {
            PageLimit::None
        } else {
            PageLimit::PageBytes
        };
        ensure_eq(
            &page.page.limited_by,
            &expected,
            "page.page.limited_by == expected",
        )?;
    }
    Ok(())
}

#[test]
fn paged_finding_read_pages_survive_an_edit_between_pages() -> TestResult {
    let manager = SourceSyncManager::default();
    let (project, path, written) = project_with_findings(&manager)?;
    let first = read(&manager, request(&path, None)).check("page one")?;
    let cursor = first.page.cursor.clone().check("several pages")?;

    // One matching row vanishes (deleted straight from the database) and
    // one new row appears.
    let vanished = written
        .last()
        .copied()
        .check("at least one finding was written")?;
    let connection = crate::store::open_findings(&findings_db_path(&project)?).check("open db")?;
    connection
        .execute("DELETE FROM findings WHERE id = ?1", [vanished])
        .check("delete a finding")?;
    drop(connection);
    let appeared = write_finding(&project, "lib.rs", "recorded between pages", &manager)?.id;
    let rest = rest_of(&manager, &first)?;

    let pages: Vec<&FindingReadResponse> = std::iter::once(&first).chain(&rest).collect();
    ensure_eq(&ids_of(&pages), &written, "ids_of(&pages) == written")?;
    ensure(
        rest.iter().all(|page| page.page.total == first.page.total),
        "rest.iter().all(|page| page.page.total == first.page.total)",
    )?;
    ensure(
        rest.iter()
            .all(|page| page.page.revision_id == first.page.revision_id),
        "rest.iter() .all(|page| page.page.revision_id == first.page.revision_id)",
    )?;
    ensure_eq(
        &first.page.cursor.as_deref(),
        &Some(cursor.as_str()),
        "first.page.cursor.as_deref() == Some(cursor.as_str())",
    )?;
    let fresh = read(&manager, request(&path, None)).check("fresh request")?;
    let fresh_rest = rest_of(&manager, &fresh)?;
    let fresh_pages: Vec<&FindingReadResponse> =
        std::iter::once(&fresh).chain(&fresh_rest).collect();
    let fresh_ids = ids_of(&fresh_pages);
    ensure(
        !fresh_ids.contains(&vanished),
        "!fresh_ids.contains(&vanished)",
    )?;
    ensure(
        fresh_ids.contains(&appeared),
        "fresh_ids.contains(&appeared)",
    )?;
    ensure_eq(
        &fresh.page.total,
        &Some(ROWS as u64),
        "fresh.page.total == Some(ROWS as u64)",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_new_finding_after_page_one_is_not_in_the_stored_result_but_in_a_fresh_request()
-> TestResult {
    let manager = SourceSyncManager::default();
    let (project, path, written) = project_with_findings(&manager)?;
    let first = read(&manager, request(&path, None)).check("page one")?;
    let appeared = write_finding(&project, "lib.rs", "late", &manager)?.id;

    let rest = rest_of(&manager, &first)?;
    let stored: Vec<&FindingReadResponse> = std::iter::once(&first).chain(&rest).collect();
    let fresh = read(&manager, request(&path, None)).check("fresh request")?;

    ensure_eq(&ids_of(&stored), &written, "ids_of(&stored) == written")?;
    ensure_eq(
        &fresh.page.total,
        &Some(ROWS as u64 + 1),
        "fresh.page.total == Some(ROWS as u64 + 1)",
    )?;
    let fresh_rest = rest_of(&manager, &fresh)?;
    let fresh_all: Vec<&FindingReadResponse> = std::iter::once(&fresh).chain(&fresh_rest).collect();
    ensure_eq(
        &ids_of(&fresh_all).last(),
        &Some(&appeared),
        "ids_of(&fresh_all).last() == Some(&appeared)",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_same_cursor_twice_returns_identical_pages() -> TestResult {
    let manager = SourceSyncManager::default();
    let (_project, path, _) = project_with_findings(&manager)?;
    let token = read(&manager, request(&path, None))
        .check("page one")?
        .page
        .cursor
        .check("several pages")?;

    let one = read(&manager, cursor_request(&token)).check("first read")?;
    let two = read(&manager, cursor_request(&token)).check("second read")?;

    ensure_eq(
        &ids_of(&[&one]),
        &ids_of(&[&two]),
        "ids_of(&[&one]) == ids_of(&[&two])",
    )?;
    ensure_eq(&one.page, &two.page, "one.page == two.page")?;
    ensure_eq(
        &serde_json::to_value(&one).check("serializes")?,
        &serde_json::to_value(&two).check("serializes")?,
        "serde_json::to_value(&one).check(\"serializes\")? == serde_json::to_value(&two).check(\"serializes\")?",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_final_page_can_be_retried() -> TestResult {
    let manager = SourceSyncManager::default();
    let (_project, path, _) = project_with_findings(&manager)?;
    let first = read(&manager, request(&path, None)).check("page one")?;
    let mut token = first.page.cursor.clone().check("several pages")?;
    let mut final_token = token.clone();
    let last = loop {
        let page = read(&manager, cursor_request(&token)).check("page")?;
        match page.page.cursor.clone() {
            Some(next) => {
                final_token = token;
                token = next;
            }
            None => break page,
        }
    };
    // `final_token` produced the page before `last`; `token` produced `last`.

    let again = read(&manager, cursor_request(&token)).check("retry of the final page")?;

    ensure_ne(&final_token, &token, "final_token != token")?;
    ensure(again.page.complete, "again.page.complete")?;
    ensure_eq(&again.page.cursor, &None, "again.page.cursor == None")?;
    ensure_eq(
        &again.page.limited_by,
        &PageLimit::None,
        "again.page.limited_by == PageLimit::None",
    )?;
    ensure_eq(&again.page, &last.page, "again.page == last.page")?;
    ensure_eq(
        &ids_of(&[&again]),
        &ids_of(&[&last]),
        "ids_of(&[&again]) == ids_of(&[&last])",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_refuses_offset_and_expected_revision_naming_cursor() -> TestResult {
    let manager = SourceSyncManager::default();
    for extra in [
        serde_json::json!({ "offset": 1 }),
        serde_json::json!({ "expected_revision": "r1" }),
    ] {
        let mut body = serde_json::json!({ "path": "/nowhere" });
        body.as_object_mut()
            .check("object")?
            .extend(extra.as_object().check("object")?.clone());
        let parsed: FindingReadRequest = serde_json::from_value(body).check("still parses")?;

        let error = read(&manager, parsed).check_err("removed parameters are refused")?;

        ensure_eq(
            &error.code,
            &rmcp::model::ErrorCode::INVALID_PARAMS,
            "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
        )?;
        ensure_eq(
            &error_code(&error),
            &Some("paging_parameter_removed"),
            "error_code(&error) == Some(\"paging_parameter_removed\")",
        )?;
        ensure(
            error.message.contains("cursor"),
            format!("{}", error.message),
        )?;
    }
    Ok(())
}

#[test]
fn paged_finding_read_without_cursor_or_path_is_refused() -> TestResult {
    let manager = SourceSyncManager::default();
    let neither = FindingReadRequest {
        path: None,
        ..request("", None)
    };

    let error = read(&manager, neither).check_err("path is required without a cursor")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &"path is required when no cursor is given",
        "error.message == \"path is required when no cursor is given\"",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_unknown_cursor_is_refused_with_unknown_cursor() -> TestResult {
    let manager = SourceSyncManager::default();

    let error = read(&manager, cursor_request("v1.not-a-real-token"))
        .check_err("a forged cursor is refused")?;

    ensure_eq(
        &error_code(&error),
        &Some("unknown_cursor"),
        "error_code(&error) == Some(\"unknown_cursor\")",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_cursor_ignores_path_and_file() -> TestResult {
    let manager = SourceSyncManager::default();
    let (_project, path, _) = project_with_findings(&manager)?;
    let token = read(&manager, request(&path, None))
        .check("page one")?
        .page
        .cursor
        .check("several pages")?;
    let plain = read(&manager, cursor_request(&token)).check("cursor only")?;

    let shaped = read(
        &manager,
        FindingReadRequest {
            path: Some("/not/a/project".to_owned()),
            file: Some("other.rs".to_owned()),
            ..cursor_request(&token)
        },
    )
    .check("path and file are ignored with a cursor")?;

    ensure_eq(
        &ids_of(&[&plain]),
        &ids_of(&[&shaped]),
        "ids_of(&[&plain]) == ids_of(&[&shaped])",
    )?;
    ensure_eq(&plain.page, &shaped.page, "plain.page == shaped.page")?;
    Ok(())
}

#[test]
fn paged_finding_read_continuation_keeps_the_page_budget_of_page_one() -> TestResult {
    let manager = SourceSyncManager::default();
    let (_project, path, _) = project_with_findings(&manager)?;
    // A budget that holds about two of the 10,000 character findings,
    // well under the process default a continuation would otherwise use.
    let _config = write_default_page_bytes(21_000)?;
    let first = read(&manager, request(&path, None)).check("page one")?;
    let cursor = first.page.cursor.clone().check("several pages")?;

    let second = read(&manager, cursor_request(&cursor)).check("page two")?;

    ensure_eq(
        &first.page.page_bytes,
        &21_000,
        "first.page.page_bytes == 21_000",
    )?;
    ensure_eq(
        &second.page.page_bytes,
        &21_000,
        "second.page.page_bytes == 21_000",
    )?;
    ensure_eq(
        &second.findings.len(),
        &first.findings.len(),
        "second.findings.len() == first.findings.len()",
    )?;
    ensure(
        second.page.bytes_used <= 21_000,
        "second.page.bytes_used <= 21_000",
    )?;
    ensure_eq(
        &second.page.limited_by,
        &PageLimit::PageBytes,
        "second.page.limited_by == PageLimit::PageBytes",
    )?;
    Ok(())
}

#[test]
fn paged_finding_read_honours_request_page_bytes_and_refuses_zero() -> TestResult {
    let manager = SourceSyncManager::default();
    let (_project, path, written) = project_with_findings(&manager)?;

    let zero_err = read(
        &manager,
        FindingReadRequest {
            page_bytes: Some(0),
            ..request(&path, None)
        },
    )
    .check_err("page_bytes = 0 must be refused")?;
    ensure_eq(
        &zero_err.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "zero page_bytes is invalid_params",
    )?;

    // Custom 21,000-byte request budget fits ~2 findings per page on page one,
    // and an explicit 32,000-byte continuation budget resizes page two to ~3.
    let first = read(
        &manager,
        FindingReadRequest {
            page_bytes: Some(21_000),
            ..request(&path, None)
        },
    )
    .check("page one with explicit page_bytes")?;
    ensure_eq(&first.page.page_bytes, &21_000, "echoes custom page_bytes")?;
    ensure_eq(
        &first.findings.len(),
        &2,
        "two 10k findings fit in 21,000 bytes",
    )?;
    ensure_eq(
        &first.page.limited_by,
        &PageLimit::PageBytes,
        "cut by page_bytes",
    )?;
    let token = first.page.cursor.clone().check("cursor present")?;

    let second = read(
        &manager,
        FindingReadRequest {
            page_bytes: Some(32_000),
            ..cursor_request(&token)
        },
    )
    .check("page two with resized page_bytes")?;
    ensure_eq(
        &second.page.page_bytes,
        &32_000,
        "continuation resizes page_bytes",
    )?;
    ensure_eq(
        &second.findings.len(),
        &3,
        "three 10k findings fit in 32,000 bytes",
    )?;
    ensure(
        written.starts_with(&ids_of(&[&first, &second])),
        "pages cover findings in id order",
    )?;
    Ok(())
}
