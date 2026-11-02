//! `mechanic list` and its cursor: the stored catalogue, the refusal of the
//! paging parameters it does not take, and the promise that a continuation never
//! registers or removes anything. The golden equality of `add` and `list` is
//! `golden_phase9_mechanic_tests`.
use super::tests::{EVAL_QUERY, action, add, call, path_of, python_project};
use super::*;
use crate::tools::PageLimit;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::collections::BTreeSet;

/// Small enough that the built-ins plus two registered entries need at least
/// three pages.
const SMALL_PAGE: usize = 250;

fn list_page(path: &str, page_bytes: usize) -> MechanicRequest {
    let mut request = action(path, MechanicAction::List, None);
    request.page_bytes = Some(page_bytes);
    request
}

fn continuation(cursor: &str, page_bytes: usize) -> MechanicRequest {
    let mut request = action("", MechanicAction::List, None);
    request.path = None;
    request.cursor = Some(cursor.to_owned());
    request.page_bytes = Some(page_bytes);
    request
}

fn ids(response: &MechanicResponse) -> Vec<String> {
    response
        .mechanics
        .iter()
        .map(|listing| listing.id.clone())
        .collect()
}

fn cursor_of(response: &MechanicResponse) -> TestResult<Option<String>> {
    Ok(response
        .page
        .as_ref()
        .check("list carries a page")?
        .cursor
        .clone())
}

/// Every page after `first`, followed by cursor only; returns the pages.
fn rest(
    manager: &sync::SourceSyncManager,
    first: &MechanicResponse,
    page_bytes: usize,
) -> TestResult<Vec<MechanicResponse>> {
    let mut pages = Vec::new();
    let mut next = cursor_of(first)?;
    while let Some(cursor) = next {
        let page = call(manager, continuation(&cursor, page_bytes)).check("continuation serves")?;
        next = cursor_of(&page)?;
        pages.push(page);
    }
    Ok(pages)
}

fn two_registered(manager: &sync::SourceSyncManager, path: &str) -> TestResult {
    for id in ["alpha", "beta"] {
        call(manager, add(path, id, "python", EVAL_QUERY)).check("register")?;
    }
    Ok(())
}

#[test]
fn paged_mechanic_list_cursor_survives_a_mechanic_added_between_pages() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path)?;
    let full = call(&manager, list_page(&path, 1 << 20)).check("full list")?;
    let first = call(&manager, list_page(&path, SMALL_PAGE)).check("page one")?;
    let revision = first.page.as_ref().check("page")?.revision_id.clone();

    call(
        &manager,
        action(&path, MechanicAction::Remove, Some("alpha")),
    )
    .check("remove alpha")?;
    call(&manager, add(&path, "gamma", "python", EVAL_QUERY)).check("add gamma")?;
    let later = rest(&manager, &first, SMALL_PAGE)?;

    assert!(later.len() >= 2, "the fixture needs at least three pages");
    let mut seen = ids(&first);
    for page in &later {
        seen.extend(ids(page));
        assert_eq!(page.page.as_ref().check("page")?.revision_id, revision);
        assert_eq!(
            page.page.as_ref().check("page")?.total,
            Some(seen_total(&full)?)
        );
    }
    assert_eq!(seen, ids(&full), "pages equal the catalogue at page one");
    assert_eq!(
        first.page.as_ref().check("page")?.total,
        Some(seen_total(&full)?)
    );
    let fresh = call(&manager, list_page(&path, 1 << 20)).check("fresh list")?;
    let fresh_ids: BTreeSet<String> = ids(&fresh).into_iter().collect();
    assert!(fresh_ids.contains("gamma") && !fresh_ids.contains("alpha"));
    Ok(())
}

fn seen_total(full: &MechanicResponse) -> TestResult<u64> {
    u64::try_from(full.mechanics.len()).check("small catalogue")
}

#[test]
fn paged_mechanic_same_cursor_twice_returns_identical_pages() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path)?;
    let first = call(&manager, list_page(&path, SMALL_PAGE)).check("page one")?;
    let cursor = cursor_of(&first)?.check("more pages remain")?;

    let a = call(&manager, continuation(&cursor, SMALL_PAGE)).check("first read")?;
    let b = call(&manager, continuation(&cursor, SMALL_PAGE)).check("second read")?;

    assert_eq!(
        serde_json::to_value(&a).check("serializes")?,
        serde_json::to_value(&b).check("serializes")?
    );
    Ok(())
}

#[test]
fn paged_mechanic_final_page_can_be_retried() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path)?;
    let first = call(&manager, list_page(&path, SMALL_PAGE)).check("page one")?;
    let mut cursor = cursor_of(&first)?.check("more pages remain")?;
    let (final_cursor, final_page) = loop {
        let page = call(&manager, continuation(&cursor, SMALL_PAGE)).check("page")?;
        match cursor_of(&page)? {
            Some(next) => cursor = next,
            None => break (cursor, page),
        }
    };

    let again = call(&manager, continuation(&final_cursor, SMALL_PAGE)).check("retry")?;

    let page = again.page.as_ref().check("page")?;
    assert!(page.complete && page.cursor.is_none());
    assert_eq!(
        serde_json::to_value(&again).check("serializes")?,
        serde_json::to_value(&final_page).check("serializes")?
    );
    Ok(())
}

#[test]
fn paged_mechanic_refuses_offset_and_expected_revision_naming_cursor() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    for field in ["offset", "expected_revision"] {
        let request: MechanicRequest = serde_json::from_value(serde_json::json!({
            "path": path, "action": "list", field: 1,
        }))
        .check("the removed field is still accepted on input")?;

        let error = call(&manager, request).check_err("removed parameter is refused")?;

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let data = error.data.check("payload")?;
        assert_eq!(data["code"], "paging_parameter_removed");
        assert!(data["remedy"].as_str().check("remedy")?.contains("cursor"));
    }
    Ok(())
}

#[test]
fn paged_mechanic_continuing_add_does_not_register_again() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path)?;
    let first = call(&manager, list_page(&path, SMALL_PAGE)).check("page one")?;
    let cursor = cursor_of(&first)?.check("more pages remain")?;
    let mut replay = add(&path, "delta", "python", EVAL_QUERY);
    replay.cursor = Some(cursor);
    replay.page_bytes = Some(SMALL_PAGE);

    let served = call(&manager, replay).check("a cursor continues the stored list")?;

    assert_eq!(served.action, MechanicAction::List);
    assert!(served.id.is_none() && served.first_count.is_none());
    let registered = manager
        .session_mechanics()
        .with_entries(|entries| entries.len())
        .check("read the session list")?;
    assert_eq!(registered, 2, "the continuation registered nothing");
    Ok(())
}

#[test]
fn paged_mechanic_list_without_path_or_cursor_is_refused() -> TestResult {
    let manager = sync::SourceSyncManager::default();
    let mut request = action("", MechanicAction::List, None);
    request.path = None;

    let error = call(&manager, request).check_err("a list needs a path or a cursor")?;

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "path is required when no cursor is given");
    Ok(())
}

#[test]
fn paged_mechanic_continuation_keeps_the_page_budget_of_page_one() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path)?;
    let _config = crate::tools::test_support::use_global_config(&format!(
        "{{\"default_page_bytes\": {SMALL_PAGE}}}"
    ))?;
    let mut request = action(&path, MechanicAction::List, None);
    request.page_bytes = None;
    let first = call(&manager, request).check("page one")?;
    let cursor = cursor_of(&first)?.check("more pages remain")?;

    let mut next = continuation(&cursor, SMALL_PAGE);
    next.page_bytes = None;
    let second = call(&manager, next).check("page two")?;

    let page = second.page.as_ref().check("list carries a page")?;
    assert_eq!(first.page.as_ref().check("page")?.page_bytes, SMALL_PAGE);
    assert_eq!(page.page_bytes, SMALL_PAGE);
    assert!(page.bytes_used <= SMALL_PAGE);
    assert_eq!(page.limited_by, PageLimit::PageBytes);
    assert!(cursor_of(&second)?.is_some(), "still several pages");
    Ok(())
}
