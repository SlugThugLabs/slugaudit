use super::*;

#[test]
fn leads_page_bytes_sets_the_row_budget_and_pages_on_it() {
    let project = unwrap_fixture(3, 4);
    let mut whole = request(project.path(), Some("unwrap_calls"));
    whole.page_bytes = Some(1_000_000);
    let whole = run(whole);
    assert_eq!(whole.page.returned, 12);
    let half = whole.page.bytes_used / 2;

    let mut paged = request(project.path(), Some("unwrap_calls"));
    paged.page_bytes = Some(half);
    let paged = run(paged);

    assert_eq!(paged.page.page_bytes, half);
    assert!(
        paged.page.returned > 0 && paged.page.returned < 12,
        "{}",
        paged.page.returned
    );
    assert!(paged.page.bytes_used <= half);
    assert_eq!(paged.page.limited_by, PageLimit::PageBytes);
    assert!(!paged.oversized_row);
    assert!(paged.page.cursor.is_some());
}

#[test]
fn leads_default_page_is_the_configured_default_page_bytes() {
    let project = unwrap_fixture(3, 4);
    let limits = ResourceLimits {
        default_page_bytes: 120,
        ..ResourceLimits::default()
    };

    let response = run_with_limits(request(project.path(), Some("unwrap_calls")), &limits);

    assert_eq!(response.page.page_bytes, 120);
    assert_eq!(response.page.limited_by, PageLimit::PageBytes);
    assert_eq!(
        response.page.total,
        Some(12),
        "counts stay complete under paging"
    );
}

#[test]
fn leads_page_bytes_above_the_ceiling_is_clamped_and_a_cut_reports_the_ceiling() {
    let project = unwrap_fixture(1, 2);
    let limits = ResourceLimits {
        max_query_response_bytes: 50,
        ..ResourceLimits::default()
    };
    let mut asked = request(project.path(), Some("unwrap_calls"));
    asked.page_bytes = Some(10_000);

    let response = run_with_limits(asked, &limits);

    assert_eq!(response.page.page_bytes, 50);
    assert_eq!(response.page.limited_by, PageLimit::Ceiling);
    assert!(!response.page.complete);
}

#[test]
fn leads_flags_a_first_row_larger_than_the_page() {
    let project = unwrap_fixture(1, 3);
    let mut asked = request(project.path(), Some("unwrap_calls"));
    asked.page_bytes = Some(1);

    let response = run(asked);

    assert_eq!(response.page.returned, 1, "one row per page minimum");
    assert!(response.oversized_row);
    assert!(response.page.cursor.is_some());
}

#[test]
fn leads_refuses_a_zero_page_bytes_instead_of_substituting() {
    let project = unwrap_fixture(1, 1);
    let mut asked = request(project.path(), Some("unwrap_calls"));
    asked.page_bytes = Some(0);

    let error = try_run(asked).expect_err("zero is a contract error");

    assert!(error.message.contains("page_bytes"), "{}", error.message);
}

#[test]
fn leads_counts_only_reply_carries_the_page_size_that_applied() {
    let project = unwrap_fixture(1, 1);

    let response = run(request(project.path(), None));

    assert_eq!(
        response.page.page_bytes,
        ResourceLimits::default().default_page_bytes
    );
    assert!(!response.oversized_row);
}

/// The byte budget replaces the deleted row cap: with no `limit`, rows stop
/// at the budget in both row modes, and the response says so rather than
/// reading as a small mechanic. One row is always returned, so an empty page
/// never means "budget too small".
#[test]
fn the_byte_budget_stops_the_page_in_both_modes_and_reports_it() {
    let project = unwrap_fixture(3, 4);
    for (mode, budget) in [(LeadMode::Full, 8), (LeadMode::Compact, 40)] {
        let limits = ResourceLimits {
            max_query_response_bytes: budget,
            ..ResourceLimits::default()
        };
        let mut request = request(project.path(), Some("unwrap_calls"));
        request.mode = mode;

        let response = run_with_limits(request, &limits);

        assert_eq!(
            response.page.returned, 1,
            "the first row is always returned"
        );
        assert!(!response.page.complete);
        // The default page is above this tiny ceiling, so the budget that
        // cut the page is the ceiling itself.
        assert_eq!(response.page.limited_by, PageLimit::Ceiling);
        assert_eq!(response.page.page_bytes, budget);
        assert!(
            response.page.bytes_used > budget,
            "{}",
            response.page.bytes_used
        );
        assert_eq!(response.page.total, Some(12), "the total is still complete");
        assert!(response.page.cursor.is_some());
    }
    // A byte-limited page cannot grow by raising `limit`; the schema's own
    // advice must name the knob that acted.
    let schema = schemars::schema_for!(PageLimit).to_value().to_string();
    assert!(
        schema.contains("page_bytes") && !schema.contains("Raise `limit`"),
        "{schema}"
    );
}
