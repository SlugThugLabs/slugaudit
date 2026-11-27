//! Continuation `page_bytes`, `limit`, and ceiling inheritance/clamping policy
//! for `leads`.

use super::paged_tests::{SMALL_PAGE, places};
use super::tests::{continuation, marker_fixture, request, run_on};
use super::*;
use crate::tools::PageLimit;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

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
