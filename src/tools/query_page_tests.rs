use super::*;

/// 500 rows of roughly 1 KiB each (`pad` is 1000 hex characters).
const FAT_ROWS: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 500) \
     SELECT x, hex(zeroblob(500)) AS pad FROM c";

fn ask_page(
    project: &tempfile::TempDir,
    sql: &str,
    page_bytes: Option<usize>,
    limits: &ResourceLimits,
) -> Result<QueryResponse, ErrorData> {
    query_with_limits(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes,
        }),
        limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn the_default_page_cuts_at_64_kib_and_carries_a_cursor() {
    let project = activated_project(&[]);
    let limits = ResourceLimits::default();

    let first = ask_page(&project, FAT_ROWS, None, &limits).expect("first page");

    assert_eq!(first.page.page_bytes, 65_536);
    assert!(!first.page.complete);
    assert!(first.page.bytes_used <= 65_536, "{}", first.page.bytes_used);
    assert!(first.rows.len() > 1 && first.rows.len() < 500);
    assert!(first.page.cursor.is_some());
    assert!(first.clipped_cells.is_empty());
    assert_eq!(first.page.limited_by, PageLimit::PageBytes);
    assert_eq!(
        first.page.total,
        Some(500),
        "the exact total travels with the page"
    );
}

#[test]
fn a_page_bytes_override_returns_a_larger_page_than_the_default() {
    let project = activated_project(&[]);
    let limits = ResourceLimits::default();
    let default_page = ask_page(&project, FAT_ROWS, None, &limits).expect("default page");

    let larger = ask_page(&project, FAT_ROWS, Some(262_144), &limits).expect("larger page");

    assert_eq!(larger.page.page_bytes, 262_144);
    assert!(larger.rows.len() > default_page.rows.len());
    assert!(larger.page.bytes_used <= 262_144);
}

#[test]
fn a_page_bytes_above_the_ceiling_is_clamped_and_the_response_says_so() {
    let project = activated_project(&[]);
    let limits = ResourceLimits {
        max_query_response_bytes: 5_000,
        ..ResourceLimits::default()
    };

    let response = ask_page(&project, FAT_ROWS, Some(1_000_000), &limits).expect("clamped");

    assert_eq!(response.page.page_bytes, 5_000);
    assert_eq!(response.page.limited_by, PageLimit::Ceiling);
    assert!(!response.page.complete);
    assert!(response.page.bytes_used <= 5_000);
}

#[test]
fn a_zero_page_bytes_is_refused_not_substituted() {
    let project = activated_project(&[]);

    let error = ask_page(&project, "SELECT 1", Some(0), &ResourceLimits::default())
        .expect_err("zero is a contract error");

    assert!(error.message.contains("page_bytes"), "{}", error.message);
}

#[test]
fn a_duplicate_result_column_name_is_refused_without_a_partial_page() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);

    let error = ask_page(
        &project,
        "SELECT * FROM files f JOIN evidence e ON e.file_id = f.id",
        None,
        &ResourceLimits::default(),
    )
    .expect_err("two `id` columns cannot share one JSON key");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message,
        "duplicate column name \"id\" in query result; alias the columns"
    );
}

#[test]
fn a_join_with_aliased_ids_returns_both_columns() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);

    let response = ask_page(
        &project,
        "SELECT f.id AS file_id, e.id AS evidence_id \
         FROM files f JOIN evidence e ON e.file_id = f.id",
        None,
        &ResourceLimits::default(),
    )
    .expect("unique column names produce distinct JSON keys");

    assert!(!response.rows.is_empty(), "the join must produce rows");
    for row in &response.rows {
        assert!(row["file_id"].is_number(), "missing files.id: {row}");
        assert!(row["evidence_id"].is_number(), "missing evidence.id: {row}");
    }
    assert!(response.page.complete);
}

#[test]
fn the_page_total_for_a_result_that_fits_one_page_is_its_row_count() {
    let project = activated_project(&[]);

    let response = ask_page(
        &project,
        "SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3",
        None,
        &ResourceLimits::default(),
    )
    .expect("small result");

    assert_eq!(response.page.total, Some(3));
    assert!(response.page.complete);
}

#[test]
fn a_first_row_larger_than_the_page_is_clipped_and_the_cut_is_reported() {
    let project = activated_project(&[]);
    let limits = ResourceLimits::default();

    let page = ask_page(&project, FAT_ROWS, Some(100), &limits).expect("oversized first row");

    assert_eq!(page.rows.len(), 1, "one row per page minimum");
    assert!(page.page.bytes_used <= 100, "{}", page.page.bytes_used);
    assert_eq!(page.clipped_cells.len(), 1);
    let cell = &page.clipped_cells[0];
    assert_eq!(
        (cell.row, cell.column.as_str(), cell.total),
        (0, "pad", 1000)
    );
    assert!(page.page.cursor.is_some());
    assert_eq!(page.page.total, Some(500));
}

#[test]
fn an_unrepresentable_wall_clock_budget_means_no_deadline_not_a_panic() {
    let project = activated_project(&[]);
    let limits = ResourceLimits {
        max_query_wall_clock: std::time::Duration::MAX,
        ..ResourceLimits::default()
    };

    let page = ask_page(&project, "SELECT 1 AS one", None, &limits)
        .expect("a budget too large for an Instant must not panic the query");

    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.page.total, Some(1));
}

#[test]
fn an_infinite_real_is_listed_as_non_finite_not_left_as_a_bare_null() {
    let project = activated_project(&[]);

    let page = ask_page(
        &project,
        "SELECT 1e999 AS big, -1e999 AS small, NULL AS missing, 1.5 AS fine",
        None,
        &ResourceLimits::default(),
    )
    .expect("query succeeds");

    assert_eq!(page.rows[0]["big"], serde_json::Value::Null);
    assert_eq!(page.rows[0]["fine"], serde_json::json!(1.5));
    let listed: Vec<(usize, &str, &str)> = page
        .non_finite_cells
        .iter()
        .map(|cell| (cell.row, cell.column.as_str(), cell.value))
        .collect();
    assert_eq!(listed, [(0, "big", "Infinity"), (0, "small", "-Infinity")]);
}

#[test]
fn a_result_with_only_finite_reals_lists_no_non_finite_cells() {
    let project = activated_project(&[]);

    let page = ask_page(
        &project,
        "SELECT 2.5 AS a, NULL AS b",
        None,
        &ResourceLimits::default(),
    )
    .expect("query succeeds");

    assert!(page.non_finite_cells.is_empty());
}

#[test]
fn a_first_row_oversized_blob_is_clipped_with_the_continuation_metadata() {
    let project = activated_project(&[]);

    let page = ask_page(
        &project,
        "SELECT zeroblob(5000) AS b",
        Some(100),
        &ResourceLimits::default(),
    )
    .expect("oversized blob row");

    assert_eq!(page.rows.len(), 1, "one row per page minimum");
    assert_eq!(page.page.bytes_used, 100);
    let cell = &page.clipped_cells[0];
    assert_eq!(
        (cell.row, cell.column.as_str(), cell.kind, cell.unit),
        (0, "b", "blob", "bytes")
    );
    assert_eq!((cell.returned, cell.total, cell.next_start), (46, 5000, 47));
    assert_eq!(
        cell.continue_with, "substr(\"b\", 47, 4954)",
        "the hint returns the remaining 4,954 bytes, not another 46-byte chunk"
    );
    let returned = page.rows[0]["b"].as_str().expect("hex string");
    assert_eq!(returned.len(), cell.returned * 2);
    assert!(returned.bytes().all(|byte| byte == b'0'));
}

#[test]
fn a_first_row_oversized_text_is_clipped_on_a_character_boundary() {
    let project = activated_project(&[]);
    let text = "日本語".repeat(100);

    let page = ask_page(
        &project,
        &format!("SELECT '{text}' AS t"),
        Some(20),
        &ResourceLimits::default(),
    )
    .expect("oversized text row");

    let cell = &page.clipped_cells[0];
    assert_eq!((cell.kind, cell.unit), ("text", "characters"));
    assert_eq!((cell.returned, cell.total, cell.next_start), (4, 300, 5));
    assert_eq!(page.page.bytes_used, 20);
    assert_eq!(page.rows[0]["t"], "日本語日");
}

#[test]
fn an_oversized_row_after_a_small_first_row_still_ends_the_page() {
    let project = activated_project(&[]);

    let page = ask_page(
        &project,
        "SELECT 'a' AS t UNION ALL SELECT hex(zeroblob(5000)) AS t",
        Some(100),
        &ResourceLimits::default(),
    )
    .expect("later oversized row");

    assert_eq!(page.rows.len(), 1, "the small first row stays intact");
    assert_eq!(page.rows[0]["t"], "a");
    assert_eq!(page.page.bytes_used, 9);
    assert!(page.clipped_cells.is_empty());
    assert!(!page.page.complete);
    assert!(page.page.cursor.is_some());
    assert_eq!(page.page.total, Some(2));
}

#[test]
fn an_escaped_text_row_is_measured_exactly_not_by_the_raw_lower_bound() {
    let project = activated_project(&[]);

    // 200 quote characters: 200 raw bytes, but 400 bytes once JSON-escaped.
    // The raw lower bound fits a 300-byte page while the exact row does not,
    // so the row is still measured exactly and clipped.
    let page = ask_page(
        &project,
        "SELECT printf('%.*c', 200, '\"') AS t",
        Some(300),
        &ResourceLimits::default(),
    )
    .expect("escape-heavy row");

    let cell = &page.clipped_cells[0];
    assert_eq!((cell.kind, cell.unit), ("text", "characters"));
    assert_eq!((cell.returned, cell.total), (146, 200));
    assert_eq!(page.page.bytes_used, 300);
    let returned = page.rows[0]["t"].as_str().expect("text cell");
    assert_eq!(returned.chars().count(), cell.returned);
    assert!(returned.chars().all(|character| character == '"'));
}
