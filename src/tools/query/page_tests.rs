use super::tests::activated_project;
use super::*;
use crate::tools::PageLimit;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

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
fn the_default_page_cuts_at_64_kib_and_carries_a_cursor() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits::default();

    let first = ask_page(&project, FAT_ROWS, None, &limits).check("first page")?;

    ensure_eq(
        &first.page.page_bytes,
        &65_536,
        "first.page.page_bytes == 65_536",
    )?;
    ensure(!first.page.complete, "!first.page.complete")?;
    ensure(
        first.page.bytes_used <= 65_536,
        format!("{}", first.page.bytes_used),
    )?;
    ensure(
        first.rows.len() > 1 && first.rows.len() < 500,
        "first.rows.len() > 1 && first.rows.len() < 500",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;
    ensure(
        first.clipped_cells.is_empty(),
        "first.clipped_cells.is_empty()",
    )?;
    ensure_eq(
        &first.page.limited_by,
        &PageLimit::PageBytes,
        "first.page.limited_by == PageLimit::PageBytes",
    )?;
    ensure_eq(
        &first.page.total,
        &Some(500),
        "the exact total travels with the page",
    )?;
    Ok(())
}

#[test]
fn a_page_bytes_override_returns_a_larger_page_than_the_default() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits::default();
    let default_page = ask_page(&project, FAT_ROWS, None, &limits).check("default page")?;

    let larger = ask_page(&project, FAT_ROWS, Some(262_144), &limits).check("larger page")?;

    ensure_eq(
        &larger.page.page_bytes,
        &262_144,
        "larger.page.page_bytes == 262_144",
    )?;
    ensure(
        larger.rows.len() > default_page.rows.len(),
        "larger.rows.len() > default_page.rows.len()",
    )?;
    ensure(
        larger.page.bytes_used <= 262_144,
        "larger.page.bytes_used <= 262_144",
    )?;
    Ok(())
}

#[test]
fn a_page_bytes_above_the_ceiling_is_clamped_and_the_response_says_so() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits {
        max_query_response_bytes: 5_000,
        ..ResourceLimits::default()
    };

    let response = ask_page(&project, FAT_ROWS, Some(1_000_000), &limits).check("clamped")?;

    ensure_eq(
        &response.page.page_bytes,
        &5_000,
        "response.page.page_bytes == 5_000",
    )?;
    ensure_eq(
        &response.page.limited_by,
        &PageLimit::Ceiling,
        "response.page.limited_by == PageLimit::Ceiling",
    )?;
    ensure(!response.page.complete, "!response.page.complete")?;
    ensure(
        response.page.bytes_used <= 5_000,
        "response.page.bytes_used <= 5_000",
    )?;
    Ok(())
}

#[test]
fn a_zero_page_bytes_is_refused_not_substituted() -> TestResult {
    let project = activated_project(&[])?;

    let error = ask_page(&project, "SELECT 1", Some(0), &ResourceLimits::default())
        .check_err("zero is a contract error")?;

    ensure(
        error.message.contains("page_bytes"),
        format!("{}", error.message),
    )?;
    Ok(())
}

#[test]
fn a_duplicate_result_column_name_is_refused_without_a_partial_page() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;

    let error = ask_page(
        &project,
        "SELECT * FROM files f JOIN evidence e ON e.file_id = f.id",
        None,
        &ResourceLimits::default(),
    )
    .check_err("two `id` columns cannot share one JSON key")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &"duplicate column name \"id\" in query result; alias the columns",
        "error.message == \"duplicate column name \\\"id\\\" in query result; alias the columns\"",
    )?;
    Ok(())
}

#[test]
fn a_join_with_aliased_ids_returns_both_columns() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;

    let response = ask_page(
        &project,
        "SELECT f.id AS file_id, e.id AS evidence_id \
         FROM files f JOIN evidence e ON e.file_id = f.id",
        None,
        &ResourceLimits::default(),
    )
    .check("unique column names produce distinct JSON keys")?;

    ensure(!response.rows.is_empty(), "the join must produce rows")?;
    for row in &response.rows {
        ensure(
            row["file_id"].is_number(),
            format!("missing files.id: {row}"),
        )?;
        ensure(
            row["evidence_id"].is_number(),
            format!("missing evidence.id: {row}"),
        )?;
    }
    ensure(response.page.complete, "response.page.complete")?;
    Ok(())
}

#[test]
fn the_page_total_for_a_result_that_fits_one_page_is_its_row_count() -> TestResult {
    let project = activated_project(&[])?;

    let response = ask_page(
        &project,
        "SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3",
        None,
        &ResourceLimits::default(),
    )
    .check("small result")?;

    ensure_eq(
        &response.page.total,
        &Some(3),
        "response.page.total == Some(3)",
    )?;
    ensure(response.page.complete, "response.page.complete")?;
    Ok(())
}

#[test]
fn a_first_row_larger_than_the_page_is_clipped_and_the_cut_is_reported() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits::default();

    let page = ask_page(&project, FAT_ROWS, Some(100), &limits).check("oversized first row")?;

    ensure_eq(&page.rows.len(), &1, "one row per page minimum")?;
    ensure(
        page.page.bytes_used <= 100,
        format!("{}", page.page.bytes_used),
    )?;
    ensure_eq(
        &page.clipped_cells.len(),
        &1,
        "page.clipped_cells.len() == 1",
    )?;
    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(
        &(cell.row, cell.column.as_str(), cell.total),
        &(0, "pad", 1000),
        "(cell.row, cell.column.as_str(), cell.total) == (0, \"pad\", 1000)",
    )?;
    page.page
        .cursor
        .as_ref()
        .check("`page.page.cursor` is present")?;
    ensure_eq(&page.page.total, &Some(500), "page.page.total == Some(500)")?;
    Ok(())
}

#[test]
fn an_unrepresentable_wall_clock_budget_means_no_deadline_not_a_panic() -> TestResult {
    let project = activated_project(&[])?;
    let limits = ResourceLimits {
        max_query_wall_clock: std::time::Duration::MAX,
        ..ResourceLimits::default()
    };

    let page = ask_page(&project, "SELECT 1 AS one", None, &limits)
        .check("a budget too large for an Instant must not panic the query")?;

    ensure_eq(&page.rows.len(), &1, "page.rows.len() == 1")?;
    ensure_eq(&page.page.total, &Some(1), "page.page.total == Some(1)")?;
    Ok(())
}

#[test]
fn an_infinite_real_is_listed_as_non_finite_not_left_as_a_bare_null() -> TestResult {
    let project = activated_project(&[])?;

    let page = ask_page(
        &project,
        "SELECT 1e999 AS big, -1e999 AS small, NULL AS missing, 1.5 AS fine",
        None,
        &ResourceLimits::default(),
    )
    .check("query succeeds")?;

    ensure_eq(
        page.rows
            .first()
            .check("page.rows[0]")?
            .get("big")
            .check("page.rows[0][\"big\"]")?,
        &serde_json::Value::Null,
        "page.rows[0][\"big\"] == serde_json::Value::Null",
    )?;
    ensure_eq(
        page.rows
            .first()
            .check("page.rows[0]")?
            .get("fine")
            .check("page.rows[0][\"fine\"]")?,
        &serde_json::json!(1.5),
        "page.rows[0][\"fine\"] == serde_json::json!(1.5)",
    )?;
    let listed: Vec<(usize, &str, &str)> = page
        .non_finite_cells
        .iter()
        .map(|cell| (cell.row, cell.column.as_str(), cell.value))
        .collect();
    ensure_eq(
        &listed,
        &[(0, "big", "Infinity"), (0, "small", "-Infinity")],
        "listed == [(0, \"big\", \"Infinity\"), (0, \"small\", \"-Infinity\")]",
    )?;
    Ok(())
}

#[test]
fn a_result_with_only_finite_reals_lists_no_non_finite_cells() -> TestResult {
    let project = activated_project(&[])?;

    let page = ask_page(
        &project,
        "SELECT 2.5 AS a, NULL AS b",
        None,
        &ResourceLimits::default(),
    )
    .check("query succeeds")?;

    ensure(
        page.non_finite_cells.is_empty(),
        "page.non_finite_cells.is_empty()",
    )?;
    Ok(())
}

#[test]
fn a_first_row_oversized_blob_is_clipped_with_the_continuation_metadata() -> TestResult {
    let project = activated_project(&[])?;

    let page = ask_page(
        &project,
        "SELECT zeroblob(5000) AS b",
        Some(100),
        &ResourceLimits::default(),
    )
    .check("oversized blob row")?;

    ensure_eq(&page.rows.len(), &1, "one row per page minimum")?;
    ensure_eq(&page.page.bytes_used, &100, "page.page.bytes_used == 100")?;
    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(
        &(cell.row, cell.column.as_str(), cell.kind, cell.unit),
        &(0, "b", "blob", "bytes"),
        "(cell.row, cell.column.as_str(), cell.kind, cell.unit) == (0, \"b\", \"blob\", \"bytes\")",
    )?;
    ensure_eq(
        &(cell.returned, cell.total, cell.next_start),
        &(46, 5000, 47),
        "(cell.returned, cell.total, cell.next_start) == (46, 5000, 47)",
    )?;
    ensure_eq(
        &cell.continue_with,
        &"substr(\"b\", 47, 4954)",
        "the hint returns the remaining 4,954 bytes, not another 46-byte chunk",
    )?;
    let returned = page
        .rows
        .first()
        .check("page.rows[0]")?
        .get("b")
        .check("page.rows[0][\"b\"]")?
        .as_str()
        .check("hex string")?;
    ensure_eq(
        &returned.len(),
        &(cell.returned * 2),
        "returned.len() == cell.returned * 2",
    )?;
    ensure(
        returned.bytes().all(|byte| byte == b'0'),
        "returned.bytes().all(|byte| byte == b'0')",
    )?;
    Ok(())
}

#[test]
fn a_first_row_oversized_text_is_clipped_on_a_character_boundary() -> TestResult {
    let project = activated_project(&[])?;
    let text = "日本語".repeat(100);

    let page = ask_page(
        &project,
        &format!("SELECT '{text}' AS t"),
        Some(20),
        &ResourceLimits::default(),
    )
    .check("oversized text row")?;

    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(
        &(cell.kind, cell.unit),
        &("text", "characters"),
        "(cell.kind, cell.unit) == (\"text\", \"characters\")",
    )?;
    ensure_eq(
        &(cell.returned, cell.total, cell.next_start),
        &(4, 300, 5),
        "(cell.returned, cell.total, cell.next_start) == (4, 300, 5)",
    )?;
    ensure_eq(&page.page.bytes_used, &20, "page.page.bytes_used == 20")?;
    ensure_eq(
        &page
            .rows
            .first()
            .check("page.rows[0]")?
            .get("t")
            .check("page.rows[0][\"t\"]")?,
        &"日本語日",
        "page.rows[0][\"t\"] == \"日本語日\"",
    )?;
    Ok(())
}

#[test]
fn an_oversized_row_after_a_small_first_row_still_ends_the_page() -> TestResult {
    let project = activated_project(&[])?;

    let page = ask_page(
        &project,
        "SELECT 'a' AS t UNION ALL SELECT hex(zeroblob(5000)) AS t",
        Some(100),
        &ResourceLimits::default(),
    )
    .check("later oversized row")?;

    ensure_eq(&page.rows.len(), &1, "the small first row stays intact")?;
    ensure_eq(
        &page
            .rows
            .first()
            .check("page.rows[0]")?
            .get("t")
            .check("page.rows[0][\"t\"]")?,
        &"a",
        "page.rows[0][\"t\"] == \"a\"",
    )?;
    ensure_eq(&page.page.bytes_used, &9, "page.page.bytes_used == 9")?;
    ensure(
        page.clipped_cells.is_empty(),
        "page.clipped_cells.is_empty()",
    )?;
    ensure(!page.page.complete, "!page.page.complete")?;
    page.page
        .cursor
        .as_ref()
        .check("`page.page.cursor` is present")?;
    ensure_eq(&page.page.total, &Some(2), "page.page.total == Some(2)")?;
    Ok(())
}

#[test]
fn an_escaped_text_row_is_measured_exactly_not_by_the_raw_lower_bound() -> TestResult {
    let project = activated_project(&[])?;

    // 200 quote characters: 200 raw bytes, but 400 bytes once JSON-escaped.
    // The raw lower bound fits a 300-byte page while the exact row does not,
    // so the row is still measured exactly and clipped.
    let page = ask_page(
        &project,
        "SELECT printf('%.*c', 200, '\"') AS t",
        Some(300),
        &ResourceLimits::default(),
    )
    .check("escape-heavy row")?;

    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(
        &(cell.kind, cell.unit),
        &("text", "characters"),
        "(cell.kind, cell.unit) == (\"text\", \"characters\")",
    )?;
    ensure_eq(
        &(cell.returned, cell.total),
        &(146, 200),
        "(cell.returned, cell.total) == (146, 200)",
    )?;
    ensure_eq(&page.page.bytes_used, &300, "page.page.bytes_used == 300")?;
    let returned = page
        .rows
        .first()
        .check("page.rows[0]")?
        .get("t")
        .check("page.rows[0][\"t\"]")?
        .as_str()
        .check("text cell")?;
    ensure_eq(
        &returned.chars().count(),
        &cell.returned,
        "returned.chars().count() == cell.returned",
    )?;
    ensure(
        returned.chars().all(|character| character == '"'),
        "returned.chars().all(|character| character == '\"')",
    )?;
    Ok(())
}
