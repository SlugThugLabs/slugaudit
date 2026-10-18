use super::*;

#[test]
fn leads_page_bytes_sets_the_row_budget_and_pages_on_it() {
    let project = unwrap_fixture(3, 4);
    let mut whole = request(project.path(), Some("unwrap_calls"));
    whole.page_bytes = Some(1_000_000);
    let whole = run(whole);
    assert_eq!(whole.returned, 12);
    let half = whole.rows_bytes / 2;

    let mut paged = request(project.path(), Some("unwrap_calls"));
    paged.page_bytes = Some(half);
    let paged = run(paged);

    assert_eq!(paged.row_budget_bytes, half);
    assert!(
        paged.returned > 0 && paged.returned < 12,
        "{}",
        paged.returned
    );
    assert!(paged.rows_bytes <= half);
    assert_eq!(paged.limited_by, PageLimit::Bytes);
    assert!(!paged.oversized_row);
    assert_eq!(paged.page_bytes_clamped_from, None);
    assert_eq!(paged.next_offset, Some(paged.returned));
}

#[test]
fn leads_default_page_is_the_configured_default_page_bytes() {
    let project = unwrap_fixture(3, 4);
    let limits = ResourceLimits {
        default_page_bytes: 120,
        ..ResourceLimits::default()
    };

    let response = run_with_limits(request(project.path(), Some("unwrap_calls")), &limits);

    assert_eq!(response.row_budget_bytes, 120);
    assert_eq!(response.limited_by, PageLimit::Bytes);
    assert_eq!(response.total, 12, "counts stay complete under paging");
}

#[test]
fn leads_page_bytes_above_the_ceiling_is_clamped_and_reported() {
    let project = unwrap_fixture(1, 2);
    let limits = ResourceLimits {
        max_query_response_bytes: 50,
        ..ResourceLimits::default()
    };
    let mut asked = request(project.path(), Some("unwrap_calls"));
    asked.page_bytes = Some(10_000);

    let response = run_with_limits(asked, &limits);

    assert_eq!(response.row_budget_bytes, 50);
    assert_eq!(response.page_bytes_clamped_from, Some(10_000));
}

#[test]
fn leads_flags_a_first_row_larger_than_the_page() {
    let project = unwrap_fixture(1, 3);
    let mut asked = request(project.path(), Some("unwrap_calls"));
    asked.page_bytes = Some(1);

    let response = run(asked);

    assert_eq!(response.returned, 1, "one row per page minimum");
    assert!(response.oversized_row);
    assert_eq!(response.next_offset, Some(1));
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
        response.row_budget_bytes,
        ResourceLimits::default().default_page_bytes
    );
    assert!(!response.oversized_row);
}
