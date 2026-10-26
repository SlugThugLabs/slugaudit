use super::*;
use crate::sync::SourceSyncManager;

const BIG: &str = "big.txt";
const LINES: usize = 78_000;

/// ~5 MB, 78k lines, multibyte characters, JSON-escaped characters and a few
/// CRLF endings.
fn big_text() -> String {
    let mut text = String::new();
    for number in 1..=LINES {
        let ending = if number % 1000 == 0 { "\r\n" } else { "\n" };
        text.push_str(&format!(
            "{number:06} héllo wörld 日本語 🦀 \"q\" \\ \t padding padding padding{ending}"
        ));
    }
    text
}

fn ask_at(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    sql: &str,
    offset: usize,
    page_bytes: Option<usize>,
) -> Result<QueryResponse, ErrorData> {
    query_with_limits(
        &Parameters(QueryRequest {
            path: project.path().to_string_lossy().into_owned(),
            sql: sql.to_owned(),
            offset,
            expected_revision: None,
            page_bytes,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|Json(response)| response)
}

fn first_string(response: &QueryResponse, column: &str) -> String {
    response.rows[0][column]
        .as_str()
        .expect("string cell")
        .to_owned()
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex digit pair"))
        .collect()
}

fn big_project() -> (tempfile::TempDir, String) {
    let text = big_text();
    let project = activated_project(&[(BIG, text.as_bytes())]);
    (project, text)
}

#[test]
fn a_5mb_file_is_walked_with_substr_hints_and_reassembles_byte_for_byte() {
    let (project, original) = big_project();
    let manager = SourceSyncManager::default();
    let select = format!("SELECT content FROM files WHERE path = '{BIG}'");

    let first = ask_at(&project, &manager, &select, 0, None).expect("first page");

    assert!(original.len() > 4_500_000 && original.matches('\n').count() == LINES);
    assert_eq!(first.clipped_cells.len(), 1);
    let cell = &first.clipped_cells[0];
    assert_eq!((cell.row, cell.column.as_str()), (0, "content"));
    assert_eq!((cell.kind, cell.unit), ("text", "characters"));
    assert_eq!(cell.total, original.chars().count());
    assert!(
        first.page.bytes_used <= first.page.page_bytes,
        "{}",
        first.page.bytes_used
    );
    assert_eq!(cell.next_start, cell.returned + 1);
    assert_eq!(
        cell.continue_with,
        format!(
            "substr(\"content\", {}, {})",
            cell.returned + 1,
            cell.total - cell.returned
        ),
        "the hint covers every remaining unit, not one more chunk"
    );
    assert_eq!(
        first_string(&first, "content").chars().count(),
        cell.returned
    );

    let mut rebuilt = first_string(&first, "content");
    let mut walked = 1;
    let mut next_start = cell.next_start;
    while next_start <= cell.total {
        let sql = format!(
            "SELECT substr(content, {next_start}, 70000) AS content FROM files WHERE path = '{BIG}'"
        );
        let page = ask_at(&project, &manager, &sql, 0, None).expect("walk page");
        let piece = first_string(&page, "content");
        let taken = piece.chars().count();
        assert!(taken > 0, "every step must advance");
        rebuilt.push_str(&piece);
        next_start += taken;
        walked += 1;
    }

    assert!(walked > 40, "walked {walked} pages");
    assert_eq!(rebuilt, original);
}

#[test]
fn the_first_pages_own_hint_returns_the_whole_rest_of_the_cell() {
    let (project, original) = big_project();
    let manager = SourceSyncManager::default();
    let select = format!("SELECT content FROM files WHERE path = '{BIG}'");
    let first = ask_at(&project, &manager, &select, 0, None).expect("first page");
    let cell = &first.clipped_cells[0];

    let sql = format!(
        "SELECT {} AS content FROM files WHERE path = '{BIG}'",
        cell.continue_with
    );
    let next = ask_at(&project, &manager, &sql, 0, Some(10_000_000)).expect("hinted page");

    let expected: String = original.chars().skip(cell.returned).collect();
    assert!(
        expected.chars().count() > cell.returned,
        "the fixture must leave more than one page-sized chunk: {} remaining after {}",
        expected.chars().count(),
        cell.returned
    );
    assert_eq!(
        first_string(&next, "content"),
        expected,
        "one run of continue_with must return the remainder, not the next chunk"
    );
}

#[test]
fn raising_page_bytes_returns_the_whole_file_unclipped() {
    let (project, original) = big_project();
    let manager = SourceSyncManager::default();
    let select = format!("SELECT content FROM files WHERE path = '{BIG}'");

    let page = ask_at(&project, &manager, &select, 0, Some(20_000_000)).expect("large page");

    assert!(page.clipped_cells.is_empty());
    assert_eq!(first_string(&page, "content"), original);
    assert!(page.page.bytes_used <= 20_000_000);
}

#[test]
fn a_blob_is_clipped_in_bytes_walked_and_reassembled_exactly() {
    let (project, original) = big_project();
    let manager = SourceSyncManager::default();
    let head: String = original.chars().take(100_000).collect();
    let blob = |start: usize| {
        format!(
            "WITH b(b) AS (SELECT CAST(substr(content, 1, 100000) AS BLOB) FROM files \
             WHERE path = '{BIG}') SELECT substr(b, {start}, 200000) AS b FROM b"
        )
    };

    let first = ask_at(&project, &manager, &blob(1), 0, None).expect("first blob page");

    let cell = &first.clipped_cells[0];
    assert_eq!(
        (cell.kind, cell.unit, cell.total),
        ("blob", "bytes", head.len())
    );
    assert_eq!(first_string(&first, "b").len(), cell.returned * 2);
    assert!(first.page.bytes_used <= first.page.page_bytes);
    let mut rebuilt = unhex(&first_string(&first, "b"));
    while rebuilt.len() < head.len() {
        let page = ask_at(&project, &manager, &blob(rebuilt.len() + 1), 0, None).expect("walk");
        let piece = unhex(&first_string(&page, "b"));
        assert!(!piece.is_empty());
        rebuilt.extend(piece);
    }
    assert_eq!(rebuilt, head.as_bytes());
}

#[test]
fn clip_units_match_sqlite_length_and_substr_for_multibyte_text_and_blobs() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();
    let text = "héllo 日本語 🦀 ünï \"q\" \\ \n end".repeat(20);
    let source = format!("SELECT '{text}' AS t, CAST('{text}' AS BLOB) AS b");

    let page = ask_at(&project, &manager, &source, 0, Some(400)).expect("clipped page");

    assert_eq!(page.clipped_cells.len(), 2);
    let (text_cell, blob_cell) = (&page.clipped_cells[0], &page.clipped_cells[1]);
    let lengths = ask_at(
        &project,
        &manager,
        &format!("SELECT length(t) AS t, length(b) AS b FROM ({source})"),
        0,
        None,
    )
    .expect("lengths");
    assert_eq!(lengths.rows[0]["t"], text_cell.total);
    assert_eq!(lengths.rows[0]["b"], blob_cell.total);
    assert_eq!(text_cell.total, text.chars().count());
    assert_eq!(blob_cell.total, text.len());
    let returned_text = first_string(&page, "t");
    let check = ask_at(
        &project,
        &manager,
        &format!(
            "SELECT substr(t, 1, {n}) = '{returned_text}' AS same, hex(substr(b, 1, {m})) AS h FROM ({source})",
            n = text_cell.returned,
            m = blob_cell.returned
        ),
        0,
        None,
    )
    .expect("prefix check");
    assert_eq!(check.rows[0]["same"], 1);
    assert_eq!(
        check.rows[0]["h"].as_str().expect("hex").to_lowercase(),
        first_string(&page, "b")
    );
    assert!(page.page.bytes_used <= 400, "{}", page.page.bytes_used);
}

#[test]
fn small_cells_stay_whole_and_the_room_is_shared_by_the_large_ones() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();
    let sql = "SELECT hex(zeroblob(2000)) AS small, hex(zeroblob(100000)) AS big, \
               replace(hex(zeroblob(100000)), '0', '\"') AS quoted";

    let page = ask_at(&project, &manager, sql, 0, Some(20_000)).expect("page");

    let columns: Vec<&str> = page
        .clipped_cells
        .iter()
        .map(|c| c.column.as_str())
        .collect();
    assert_eq!(columns, ["big", "quoted"]);
    assert_eq!(first_string(&page, "small").len(), 4000);
    assert!(
        page.page.bytes_used <= 20_000,
        "escapes counted: {}",
        page.page.bytes_used
    );
    assert!(
        page.page.bytes_used > 19_000,
        "room is used: {}",
        page.page.bytes_used
    );
}

#[test]
fn a_page_smaller_than_one_character_still_advances_by_one() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();

    let page = ask_at(&project, &manager, "SELECT '日本語' AS t", 0, Some(1)).expect("page");

    let cell = &page.clipped_cells[0];
    assert_eq!((cell.returned, cell.next_start, cell.total), (1, 2, 3));
    assert_eq!(first_string(&page, "t"), "日");
}

#[test]
fn the_clipped_row_index_is_absolute_and_later_rows_are_not_clipped() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();
    let rows = "SELECT n, hex(zeroblob(5000)) AS h FROM (SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3)";

    let resumed = ask_at(&project, &manager, rows, 2, Some(100)).expect("offset page");
    let big_after_small = ask_at(
        &project,
        &manager,
        "SELECT 'a' AS t UNION ALL SELECT hex(zeroblob(5000))",
        0,
        Some(100),
    )
    .expect("later row");

    assert_eq!(resumed.clipped_cells[0].row, 2);
    assert!(big_after_small.clipped_cells.is_empty());
    assert!(!big_after_small.page.complete);
    assert_eq!(big_after_small.page.next_offset, Some(1));
}

#[test]
fn line_functions_return_lines_of_the_big_file_through_query() {
    let (project, original) = big_project();
    let manager = SourceSyncManager::default();
    let sql = format!(
        "SELECT line_count(content) AS n, line_range(content, 40000, 40200) AS r \
         FROM files WHERE path = '{BIG}'"
    );
    let lines: Vec<&str> = original.split('\n').collect();

    let page = ask_at(&project, &manager, &sql, 0, Some(1_000_000)).expect("line functions");

    assert_eq!(page.rows[0]["n"], LINES);
    assert_eq!(first_string(&page, "r"), lines[39_999..40_200].join("\n"));
    assert!(page.clipped_cells.is_empty());
}

#[test]
fn a_clipped_line_range_continues_with_the_same_hint_mechanism() {
    let (project, original) = big_project();
    let manager = SourceSyncManager::default();
    let sql = format!("SELECT line_range(content, 1, 78000) AS r FROM files WHERE path = '{BIG}'");

    let page = ask_at(&project, &manager, &sql, 0, None).expect("clipped range");

    let cell = &page.clipped_cells[0];
    assert_eq!(cell.column, "r");
    assert_eq!(
        cell.total,
        original.trim_end_matches(['\r', '\n']).chars().count()
    );
}

#[test]
fn a_bad_line_range_is_an_invalid_params_error() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();

    let error = ask_at(&project, &manager, "SELECT line_range('a', 0, 1)", 0, None)
        .expect_err("start 0 is refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("start must be at least 1"),
        "{}",
        error.message
    );
}

#[test]
fn a_text_with_a_nul_is_continued_as_bytes_because_sqlite_text_functions_stop_at_it() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();
    let source = "SELECT 'ab' || char(0) || 'cdefghij' AS t";

    let page = ask_at(&project, &manager, source, 0, Some(14)).expect("clipped page");

    let cell = &page.clipped_cells[0];
    assert_eq!((cell.kind, cell.unit, cell.total), ("text", "bytes", 11));
    let rest = ask_at(
        &project,
        &manager,
        &format!("SELECT hex({}) AS h FROM ({source})", cell.continue_with),
        0,
        None,
    )
    .expect("continuation");
    let expected: String = "ab\0cdefghij"
        .bytes()
        .skip(cell.returned)
        .map(|byte| format!("{byte:02X}"))
        .collect();
    assert_eq!(first_string(&rest, "h"), expected);
}

#[test]
fn a_duplicate_column_name_is_refused_before_a_row_is_clipped() {
    let project = activated_project(&[]);
    let manager = SourceSyncManager::default();
    // The projection is refused before any row is read, so a first row that
    // would otherwise have to be clipped never reaches the clipper.
    let sql = "SELECT zeroblob(5000) AS id, zeroblob(5000) AS id";

    let error = ask_at(&project, &manager, sql, 0, Some(100))
        .expect_err("the row cannot carry two `id` columns");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message,
        "duplicate column name \"id\" in query result; alias the columns"
    );
}

#[test]
fn both_row_builders_reject_duplicate_column_names() {
    let names = vec!["id".to_owned(), "id".to_owned()];
    let values = [
        rusqlite::types::Value::Integer(1),
        rusqlite::types::Value::Integer(2),
    ];

    let plain = crate::tools::query_value::row_to_json(&names, &values)
        .expect_err("the unclipped builder cannot key two columns by one name");
    let clipped = crate::tools::query_clip::clip_row(&names, &values, 100, 0)
        .err()
        .expect("the clipped builder cannot key two columns by one name");

    for error in [plain, clipped] {
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(
            error.message,
            "duplicate column name \"id\" in query result; alias the columns"
        );
    }
}
