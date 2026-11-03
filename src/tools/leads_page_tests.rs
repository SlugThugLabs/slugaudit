use super::*;
use slugaudit_test_check::{CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn leads_page_bytes_sets_the_row_budget_and_pages_on_it() -> TestResult {
    let project = marker_fixture(3, 4)?;
    let mut whole = request(project.path(), Some("comment_markers"));
    whole.page_bytes = Some(1_000_000);
    let whole = run(whole)?;
    ensure_eq(&whole.page.returned, &12, "whole.page.returned == 12")?;
    let half = whole.page.bytes_used / 2;

    let mut paged = request(project.path(), Some("comment_markers"));
    paged.page_bytes = Some(half);
    let paged = run(paged)?;

    ensure_eq(
        &paged.page.page_bytes,
        &half,
        "paged.page.page_bytes == half",
    )?;
    ensure(
        paged.page.returned > 0 && paged.page.returned < 12,
        format!("{}", paged.page.returned),
    )?;
    ensure(
        paged.page.bytes_used <= half,
        "paged.page.bytes_used <= half",
    )?;
    ensure_eq(
        &paged.page.limited_by,
        &PageLimit::PageBytes,
        "paged.page.limited_by == PageLimit::PageBytes",
    )?;
    ensure(!paged.oversized_row, "!paged.oversized_row")?;
    paged
        .page
        .cursor
        .as_ref()
        .check("`paged.page.cursor` is present")?;
    Ok(())
}

#[test]
fn leads_default_page_is_the_configured_default_page_bytes() -> TestResult {
    let project = marker_fixture(3, 4)?;
    let limits = ResourceLimits {
        default_page_bytes: 120,
        ..ResourceLimits::default()
    };

    let response = run_with_limits(request(project.path(), Some("comment_markers")), &limits)?;

    ensure_eq(
        &response.page.page_bytes,
        &120,
        "response.page.page_bytes == 120",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::PageBytes,
        "response.page.limited_by == PageLimit::PageBytes",
    )?;
    ensure_eq(
        &response.page.total,
        &Some(12),
        "response.page.total == Some(12)",
    )?;
    Ok(())
}

#[test]
fn leads_page_bytes_above_the_ceiling_is_clamped_and_a_cut_reports_the_ceiling() -> TestResult {
    let project = marker_fixture(1, 2)?;
    let limits = ResourceLimits {
        max_query_response_bytes: 50,
        ..ResourceLimits::default()
    };
    let mut asked = request(project.path(), Some("comment_markers"));
    asked.page_bytes = Some(10_000);

    let response = run_with_limits(asked, &limits)?;

    ensure_eq(
        &response.page.page_bytes,
        &50,
        "response.page.page_bytes == 50",
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
fn leads_flags_a_first_row_larger_than_the_page() -> TestResult {
    let project = marker_fixture(1, 3)?;
    let mut asked = request(project.path(), Some("comment_markers"));
    asked.page_bytes = Some(1);

    let response = run(asked)?;

    ensure_eq(&response.page.returned, &1, "response.page.returned == 1")?;
    ensure(response.oversized_row, "response.oversized_row")?;
    response
        .page
        .cursor
        .as_ref()
        .check("`response.page.cursor` is present")?;
    Ok(())
}

#[test]
fn leads_refuses_a_zero_page_bytes_instead_of_substituting() -> TestResult {
    let project = marker_fixture(1, 1)?;
    let mut asked = request(project.path(), Some("comment_markers"));
    asked.page_bytes = Some(0);

    let error = try_run(asked).check_err("zero is a contract error")?;

    ensure(
        error.message.contains("page_bytes"),
        format!("{}", error.message),
    )?;
    Ok(())
}

#[test]
fn leads_counts_only_reply_carries_the_page_size_that_applied() -> TestResult {
    let project = marker_fixture(1, 1)?;

    let response = run(request(project.path(), None))?;

    ensure_eq(
        &response.page.page_bytes,
        &ResourceLimits::default().default_page_bytes,
        "response.page.page_bytes == ResourceLimits::default().default_page_bytes",
    )?;
    ensure(!response.oversized_row, "!response.oversized_row")?;
    Ok(())
}

/// The byte budget, not a row cap, bounds a page: with no `limit`, rows stop
/// at the budget in both row modes, and the response says so rather than
/// reading as a small mechanic. One row is always returned, so an empty page
/// never means "budget too small".
#[test]
fn the_byte_budget_stops_the_page_in_both_modes_and_reports_it() -> TestResult {
    let project = marker_fixture(3, 4)?;
    for (mode, budget) in [(LeadMode::Full, 8), (LeadMode::Compact, 40)] {
        let limits = ResourceLimits {
            max_query_response_bytes: budget,
            ..ResourceLimits::default()
        };
        let mut request = request(project.path(), Some("comment_markers"));
        request.mode = mode;

        let response = run_with_limits(request, &limits)?;

        ensure_eq(&response.page.returned, &1, "response.page.returned == 1")?;
        ensure(!response.page.complete, "!response.page.complete")?;
        // The default page is above this tiny ceiling, so the budget that
        // cut the page is the ceiling itself.
        ensure_eq(
            &response.page.limited_by,
            &PageLimit::Ceiling,
            "response.page.limited_by == PageLimit::Ceiling",
        )?;
        ensure_eq(
            &response.page.page_bytes,
            &budget,
            "response.page.page_bytes == budget",
        )?;
        ensure(
            response.page.bytes_used > budget,
            format!("{}", response.page.bytes_used),
        )?;
        ensure_eq(
            &response.page.total,
            &Some(12),
            "response.page.total == Some(12)",
        )?;
        response
            .page
            .cursor
            .as_ref()
            .check("`response.page.cursor` is present")?;
    }
    // A byte-limited page cannot grow by raising `limit`; the schema's own
    // advice must name the knob that acted.
    let schema = schemars::schema_for!(PageLimit).to_value().to_string();
    ensure(
        schema.contains("page_bytes") && !schema.contains("Raise `limit`"),
        "schema.contains(\"page_bytes\") && !schema.contains(\"Raise `limit`\")",
    )?;
    Ok(())
}
