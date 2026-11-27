use super::tests::activated_project;
use super::*;
use crate::sync::SourceSyncManager;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fmt::Write as _;

pub(super) const BIG: &str = "big.txt";
pub(super) const LINES: usize = 78_000;

/// ~5 MB, 78k lines, multibyte characters, JSON-escaped characters and a few
/// CRLF endings.
fn big_text() -> TestResult<String> {
    let mut text = String::new();
    for number in 1..=LINES {
        let ending = if number % 1000 == 0 { "\r\n" } else { "\n" };
        write!(
            text,
            "{number:06} héllo wörld 日本語 🦀 \"q\" \\ \t padding padding padding{ending}"
        )
        .check("writing to a String cannot fail")?;
    }
    Ok(text)
}

/// Continues a stored result by its cursor; the cursor alone names it.
fn ask_cursor(
    manager: &SourceSyncManager,
    cursor: Option<&str>,
    page_bytes: Option<usize>,
) -> Result<QueryResponse, ErrorData> {
    query_with_limits(
        &Parameters(QueryRequest {
            path: None,
            sql: String::new(),
            cursor: cursor.map(str::to_owned),
            legacy_paging: LegacyPaging::default(),
            page_bytes,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
}

pub(super) fn ask_at(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    sql: &str,
    page_bytes: Option<usize>,
) -> Result<QueryResponse, ErrorData> {
    query_with_limits(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
}

pub(super) fn first_string(response: &QueryResponse, column: &str) -> TestResult<String> {
    Ok(response.rows.first().check("response.rows[0]")?[column]
        .as_str()
        .check("string cell")?
        .to_owned())
}

fn unhex(hex: &str) -> TestResult<Vec<u8>> {
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        ensure_eq(&pair.len(), &2, "every hex digit pair has two digits")?;
        let digits = std::str::from_utf8(pair).check("hex digits are ASCII")?;
        bytes.push(u8::from_str_radix(digits, 16).check("hex digit pair")?);
    }
    Ok(bytes)
}

pub(super) fn big_project() -> TestResult<(tempfile::TempDir, String)> {
    let text = big_text()?;
    let project = activated_project(&[(BIG, text.as_bytes())])?;
    Ok((project, text))
}

#[test]
fn a_5mb_file_is_walked_with_substr_hints_and_reassembles_byte_for_byte() -> TestResult {
    let (project, original) = big_project()?;
    let manager = SourceSyncManager::default();
    let select = format!("SELECT content FROM files WHERE path = '{BIG}'");

    let first = ask_at(&project, &manager, &select, None).check("first page")?;

    ensure(
        original.len() > 4_500_000 && original.matches('\n').count() == LINES,
        "original.len() > 4_500_000 && original.matches('\\n').count() == LINES",
    )?;
    ensure_eq(
        &first.clipped_cells.len(),
        &1,
        "first.clipped_cells.len() == 1",
    )?;
    let cell = &first
        .clipped_cells
        .first()
        .check("first.clipped_cells[0]")?;
    ensure_eq(
        &(cell.row, cell.column.as_str()),
        &(0, "content"),
        "(cell.row, cell.column.as_str()) == (0, \"content\")",
    )?;
    ensure_eq(
        &(cell.kind, cell.unit),
        &("text", "characters"),
        "(cell.kind, cell.unit) == (\"text\", \"characters\")",
    )?;
    ensure_eq(
        &cell.total,
        &original.chars().count(),
        "cell.total == original.chars().count()",
    )?;
    ensure(
        first.page.bytes_used <= first.page.page_bytes,
        format!("{}", first.page.bytes_used),
    )?;
    ensure_eq(
        &cell.next_start,
        &(cell.returned + 1),
        "cell.next_start == cell.returned + 1",
    )?;
    ensure_eq(
        &cell.continue_with,
        &format!(
            "substr(\"content\", {}, {})",
            cell.returned + 1,
            cell.total - cell.returned
        ),
        "the hint covers every remaining unit, not one more chunk",
    )?;
    ensure_eq(
        &first_string(&first, "content")?.chars().count(),
        &cell.returned,
        "first_string(&first, \"content\")?.chars().count() == cell.returned",
    )?;

    let mut rebuilt = first_string(&first, "content")?;
    let mut walked = 1;
    let mut next_start = cell.next_start;
    while next_start <= cell.total {
        let sql = format!(
            "SELECT substr(content, {next_start}, 70000) AS content FROM files WHERE path = '{BIG}'"
        );
        let page = ask_at(&project, &manager, &sql, None).check("walk page")?;
        let piece = first_string(&page, "content")?;
        let taken = piece.chars().count();
        ensure(taken > 0, "every step must advance")?;
        rebuilt.push_str(&piece);
        next_start += taken;
        walked += 1;
    }

    ensure(walked > 40, format!("walked {walked} pages"))?;
    ensure_eq(&rebuilt, &original, "rebuilt == original")?;
    Ok(())
}

#[test]
fn the_first_pages_own_hint_returns_the_whole_rest_of_the_cell() -> TestResult {
    let (project, original) = big_project()?;
    let manager = SourceSyncManager::default();
    let select = format!("SELECT content FROM files WHERE path = '{BIG}'");
    let first = ask_at(&project, &manager, &select, None).check("first page")?;
    let cell = &first
        .clipped_cells
        .first()
        .check("first.clipped_cells[0]")?;

    let sql = format!(
        "SELECT {} AS content FROM files WHERE path = '{BIG}'",
        cell.continue_with
    );
    let next = ask_at(&project, &manager, &sql, Some(10_000_000)).check("hinted page")?;

    let expected: String = original.chars().skip(cell.returned).collect();
    ensure(
        expected.chars().count() > cell.returned,
        format!(
            "the fixture must leave more than one page-sized chunk: {} remaining after {}",
            expected.chars().count(),
            cell.returned
        ),
    )?;
    ensure_eq(
        &first_string(&next, "content")?,
        &expected,
        "one run of continue_with must return the remainder, not the next chunk",
    )?;
    Ok(())
}

#[test]
fn raising_page_bytes_returns_the_whole_file_unclipped() -> TestResult {
    let (project, original) = big_project()?;
    let manager = SourceSyncManager::default();
    let select = format!("SELECT content FROM files WHERE path = '{BIG}'");

    let page = ask_at(&project, &manager, &select, Some(20_000_000)).check("large page")?;

    ensure(
        page.clipped_cells.is_empty(),
        "page.clipped_cells.is_empty()",
    )?;
    ensure_eq(
        &first_string(&page, "content")?,
        &original,
        "first_string(&page, \"content\")? == original",
    )?;
    ensure(
        page.page.bytes_used <= 20_000_000,
        "page.page.bytes_used <= 20_000_000",
    )?;
    Ok(())
}

#[test]
fn a_blob_is_clipped_in_bytes_walked_and_reassembled_exactly() -> TestResult {
    let (project, original) = big_project()?;
    let manager = SourceSyncManager::default();
    let head: String = original.chars().take(100_000).collect();
    let blob = |start: usize| {
        format!(
            "WITH b(b) AS (SELECT CAST(substr(content, 1, 100000) AS BLOB) FROM files \
             WHERE path = '{BIG}') SELECT substr(b, {start}, 200000) AS b FROM b"
        )
    };

    let first = ask_at(&project, &manager, &blob(1), None).check("first blob page")?;

    let cell = &first
        .clipped_cells
        .first()
        .check("first.clipped_cells[0]")?;
    ensure_eq(
        &(cell.kind, cell.unit, cell.total),
        &("blob", "bytes", head.len()),
        "(cell.kind, cell.unit, cell.total) == (\"blob\", \"bytes\", head.len())",
    )?;
    ensure_eq(
        &first_string(&first, "b")?.len(),
        &(cell.returned * 2),
        "first_string(&first, \"b\")?.len() == cell.returned * 2",
    )?;
    ensure(
        first.page.bytes_used <= first.page.page_bytes,
        "first.page.bytes_used <= first.page.page_bytes",
    )?;
    let mut rebuilt = unhex(&first_string(&first, "b")?)?;
    while rebuilt.len() < head.len() {
        let page = ask_at(&project, &manager, &blob(rebuilt.len() + 1), None).check("walk")?;
        let piece = unhex(&first_string(&page, "b")?)?;
        ensure(!piece.is_empty(), "!piece.is_empty()")?;
        rebuilt.extend(piece);
    }
    ensure_eq(&rebuilt, &head.as_bytes(), "rebuilt == head.as_bytes()")?;
    Ok(())
}

#[test]
fn clip_units_match_sqlite_length_and_substr_for_multibyte_text_and_blobs() -> TestResult {
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();
    let text = "héllo 日本語 🦀 ünï \"q\" \\ \n end".repeat(20);
    let source = format!("SELECT '{text}' AS t, CAST('{text}' AS BLOB) AS b");

    let page = ask_at(&project, &manager, &source, Some(400)).check("clipped page")?;

    ensure_eq(
        &page.clipped_cells.len(),
        &2,
        "page.clipped_cells.len() == 2",
    )?;
    let (text_cell, blob_cell) = (
        &page.clipped_cells.first().check("page.clipped_cells[0]")?,
        &page.clipped_cells.get(1).check("page.clipped_cells[1]")?,
    );
    let lengths = ask_at(
        &project,
        &manager,
        &format!("SELECT length(t) AS t, length(b) AS b FROM ({source})"),
        None,
    )
    .check("lengths")?;
    ensure_eq(
        &lengths
            .rows
            .first()
            .check("lengths.rows[0]")?
            .get("t")
            .check("lengths.rows[0][\"t\"]")?,
        &text_cell.total,
        "lengths.rows[0][\"t\"] == text_cell.total",
    )?;
    ensure_eq(
        &lengths
            .rows
            .first()
            .check("lengths.rows[0]")?
            .get("b")
            .check("lengths.rows[0][\"b\"]")?,
        &blob_cell.total,
        "lengths.rows[0][\"b\"] == blob_cell.total",
    )?;
    ensure_eq(
        &text_cell.total,
        &text.chars().count(),
        "text_cell.total == text.chars().count()",
    )?;
    ensure_eq(
        &blob_cell.total,
        &text.len(),
        "blob_cell.total == text.len()",
    )?;
    let returned_text = first_string(&page, "t")?;
    let check = ask_at(
        &project,
        &manager,
        &format!(
            "SELECT substr(t, 1, {n}) = '{returned_text}' AS same, hex(substr(b, 1, {m})) AS h FROM ({source})",
            n = text_cell.returned,
            m = blob_cell.returned
        ),
        None,
    )
    .check("prefix check")?;
    ensure_eq(
        &check
            .rows
            .first()
            .check("check.rows[0]")?
            .get("same")
            .check("check.rows[0][\"same\"]")?,
        &1,
        "check.rows[0][\"same\"] == 1",
    )?;
    ensure_eq(
        &check
            .rows
            .first()
            .check("check.rows[0]")?
            .get("h")
            .check("check.rows[0][\"h\"]")?
            .as_str()
            .check("hex")?
            .to_lowercase(),
        &first_string(&page, "b")?,
        "check.rows[0][\"h\"].as_str().check(\"hex\")?.to_lowercase() == first_string(&page, \"b\")?",
    )?;
    ensure(
        page.page.bytes_used <= 400,
        format!("{}", page.page.bytes_used),
    )?;
    Ok(())
}

#[test]
fn small_cells_stay_whole_and_the_room_is_shared_by_the_large_ones() -> TestResult {
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();
    let sql = "SELECT hex(zeroblob(2000)) AS small, hex(zeroblob(100000)) AS big, \
               replace(hex(zeroblob(100000)), '0', '\"') AS quoted";

    let page = ask_at(&project, &manager, sql, Some(20_000)).check("page")?;

    let columns: Vec<&str> = page
        .clipped_cells
        .iter()
        .map(|c| c.column.as_str())
        .collect();
    ensure_eq(
        &columns,
        &["big", "quoted"],
        "columns == [\"big\", \"quoted\"]",
    )?;
    ensure_eq(
        &first_string(&page, "small")?.len(),
        &4000,
        "first_string(&page, \"small\")?.len() == 4000",
    )?;
    ensure(
        page.page.bytes_used <= 20_000,
        format!("escapes counted: {}", page.page.bytes_used),
    )?;
    ensure(
        page.page.bytes_used > 19_000,
        format!("room is used: {}", page.page.bytes_used),
    )?;
    Ok(())
}

#[test]
fn a_page_smaller_than_one_character_still_advances_by_one() -> TestResult {
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();

    let page = ask_at(&project, &manager, "SELECT '日本語' AS t", Some(1)).check("page")?;

    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(
        &(cell.returned, cell.next_start, cell.total),
        &(1, 2, 3),
        "(cell.returned, cell.next_start, cell.total) == (1, 2, 3)",
    )?;
    ensure_eq(
        &first_string(&page, "t")?,
        &"日",
        "first_string(&page, \"t\")? == \"日\"",
    )?;
    Ok(())
}

#[test]
fn the_clipped_row_index_is_absolute_and_later_rows_are_not_clipped() -> TestResult {
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();
    let rows = "SELECT n, hex(zeroblob(5000)) AS h FROM (SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3)";

    let first = ask_at(&project, &manager, rows, Some(100)).check("first page")?;
    let second = ask_cursor(&manager, first.page.cursor.as_deref(), Some(100)).check("page two")?;
    let resumed =
        ask_cursor(&manager, second.page.cursor.as_deref(), Some(100)).check("page three")?;
    let big_after_small = ask_at(
        &project,
        &manager,
        "SELECT 'a' AS t UNION ALL SELECT hex(zeroblob(5000))",
        Some(100),
    )
    .check("later row")?;

    ensure_eq(
        &resumed
            .clipped_cells
            .first()
            .check("resumed.clipped_cells[0]")?
            .row,
        &2,
        "resumed.clipped_cells[0].row == 2",
    )?;
    ensure(
        big_after_small.clipped_cells.is_empty(),
        "big_after_small.clipped_cells.is_empty()",
    )?;
    ensure(
        !big_after_small.page.complete,
        "!big_after_small.page.complete",
    )?;
    big_after_small
        .page
        .cursor
        .as_ref()
        .check("`big_after_small.page.cursor` is present")?;
    Ok(())
}
