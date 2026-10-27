//! `mechanic list` and its cursor: the stored catalogue, the refusal of the
//! removed paging parameters, and the promise that a continuation never
//! registers or removes anything. The golden equality of `add` and `list` is
//! `golden_phase9_mechanic_tests`.
use super::tests::{EVAL_QUERY, action, add, call, path_of, python_project};
use super::*;
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

fn cursor_of(response: &MechanicResponse) -> Option<String> {
    response
        .page
        .as_ref()
        .expect("list carries a page")
        .cursor
        .clone()
}

/// Every page after `first`, followed by cursor only; returns the pages.
fn rest(
    manager: &sync::SourceSyncManager,
    first: &MechanicResponse,
    page_bytes: usize,
) -> Vec<MechanicResponse> {
    let mut pages = Vec::new();
    let mut next = cursor_of(first);
    while let Some(cursor) = next {
        let page = call(manager, continuation(&cursor, page_bytes)).expect("continuation serves");
        next = cursor_of(&page);
        pages.push(page);
    }
    pages
}

fn two_registered(manager: &sync::SourceSyncManager, path: &str) {
    for id in ["alpha", "beta"] {
        call(manager, add(path, id, "python", EVAL_QUERY)).expect("register");
    }
}

#[test]
fn paged_mechanic_list_cursor_survives_a_mechanic_added_between_pages() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path);
    let full = call(&manager, list_page(&path, 1 << 20)).expect("full list");
    let first = call(&manager, list_page(&path, SMALL_PAGE)).expect("page one");
    let revision = first.page.as_ref().expect("page").revision_id.clone();

    call(
        &manager,
        action(&path, MechanicAction::Remove, Some("alpha")),
    )
    .expect("remove alpha");
    call(&manager, add(&path, "gamma", "python", EVAL_QUERY)).expect("add gamma");
    let later = rest(&manager, &first, SMALL_PAGE);

    assert!(later.len() >= 2, "the fixture needs at least three pages");
    let mut seen = ids(&first);
    for page in &later {
        seen.extend(ids(page));
        assert_eq!(page.page.as_ref().expect("page").revision_id, revision);
        assert_eq!(
            page.page.as_ref().expect("page").total,
            Some(seen_total(&full))
        );
    }
    assert_eq!(seen, ids(&full), "pages equal the catalogue at page one");
    assert_eq!(
        first.page.as_ref().expect("page").total,
        Some(seen_total(&full))
    );
    let fresh = call(&manager, list_page(&path, 1 << 20)).expect("fresh list");
    let fresh_ids: BTreeSet<String> = ids(&fresh).into_iter().collect();
    assert!(fresh_ids.contains("gamma") && !fresh_ids.contains("alpha"));
}

fn seen_total(full: &MechanicResponse) -> u64 {
    u64::try_from(full.mechanics.len()).expect("small catalogue")
}

#[test]
fn paged_mechanic_same_cursor_twice_returns_identical_pages() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path);
    let first = call(&manager, list_page(&path, SMALL_PAGE)).expect("page one");
    let cursor = cursor_of(&first).expect("more pages remain");

    let a = call(&manager, continuation(&cursor, SMALL_PAGE)).expect("first read");
    let b = call(&manager, continuation(&cursor, SMALL_PAGE)).expect("second read");

    assert_eq!(
        serde_json::to_value(&a).expect("serializes"),
        serde_json::to_value(&b).expect("serializes")
    );
}

#[test]
fn paged_mechanic_final_page_can_be_retried() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path);
    let first = call(&manager, list_page(&path, SMALL_PAGE)).expect("page one");
    let mut cursor = cursor_of(&first).expect("more pages remain");
    let (final_cursor, final_page) = loop {
        let page = call(&manager, continuation(&cursor, SMALL_PAGE)).expect("page");
        match cursor_of(&page) {
            Some(next) => cursor = next,
            None => break (cursor, page),
        }
    };

    let again = call(&manager, continuation(&final_cursor, SMALL_PAGE)).expect("retry");

    let page = again.page.as_ref().expect("page");
    assert!(page.complete && page.cursor.is_none());
    assert_eq!(
        serde_json::to_value(&again).expect("serializes"),
        serde_json::to_value(&final_page).expect("serializes")
    );
}

#[test]
fn paged_mechanic_refuses_offset_and_expected_revision_naming_cursor() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    for field in ["offset", "expected_revision"] {
        let request: MechanicRequest = serde_json::from_value(serde_json::json!({
            "path": path, "action": "list", field: 1,
        }))
        .expect("the removed field is still accepted on input");

        let error = call(&manager, request).expect_err("removed parameter is refused");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let data = error.data.expect("payload");
        assert_eq!(data["code"], "paging_parameter_removed");
        assert!(data["remedy"].as_str().expect("remedy").contains("cursor"));
    }
}

#[test]
fn paged_mechanic_continuing_add_does_not_register_again() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    two_registered(&manager, &path);
    let first = call(&manager, list_page(&path, SMALL_PAGE)).expect("page one");
    let cursor = cursor_of(&first).expect("more pages remain");
    let mut replay = add(&path, "delta", "python", EVAL_QUERY);
    replay.cursor = Some(cursor);
    replay.page_bytes = Some(SMALL_PAGE);

    let served = call(&manager, replay).expect("a cursor continues the stored list");

    assert_eq!(served.action, MechanicAction::List);
    assert!(served.id.is_none() && served.first_count.is_none());
    let registered = manager
        .session_mechanics()
        .with_entries(|entries| Ok::<_, ErrorData>(entries.len()))
        .expect("read the session list");
    assert_eq!(registered, 2, "the continuation registered nothing");
}

#[test]
fn paged_mechanic_list_without_path_or_cursor_is_refused() {
    let manager = sync::SourceSyncManager::default();
    let mut request = action("", MechanicAction::List, None);
    request.path = None;

    let error = call(&manager, request).expect_err("a list needs a path or a cursor");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "path is required when no cursor is given");
}
