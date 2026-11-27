//! Line-range, NUL-text and duplicate-column behaviour of clipped query rows.
use super::clip_tests::{BIG, LINES, ask_at, big_project, first_string};
use super::tests::activated_project;
use crate::sync::SourceSyncManager;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn line_functions_return_lines_of_the_big_file_through_query() -> TestResult {
    let (project, original) = big_project()?;
    let manager = SourceSyncManager::default();
    let sql = format!(
        "SELECT line_count(content) AS n, line_range(content, 40000, 40200) AS r \
         FROM files WHERE path = '{BIG}'"
    );
    let lines: Vec<&str> = original.split('\n').collect();

    let page = ask_at(&project, &manager, &sql, Some(1_000_000)).check("line functions")?;

    ensure_eq(
        &page
            .rows
            .first()
            .check("page.rows[0]")?
            .get("n")
            .check("page.rows[0][\"n\"]")?,
        &LINES,
        "page.rows[0][\"n\"] == LINES",
    )?;
    ensure_eq(
        &first_string(&page, "r")?,
        &lines
            .get(39_999..40_200)
            .check("lines[39_999..40_200]")?
            .join("\n"),
        "first_string(&page, \"r\")? == lines[39_999..40_200].join(\"\\n\")",
    )?;
    ensure(
        page.clipped_cells.is_empty(),
        "page.clipped_cells.is_empty()",
    )?;
    Ok(())
}

#[test]
fn a_clipped_line_range_continues_with_the_same_hint_mechanism() -> TestResult {
    let (project, original) = big_project()?;
    let manager = SourceSyncManager::default();
    let sql = format!("SELECT line_range(content, 1, 78000) AS r FROM files WHERE path = '{BIG}'");

    let page = ask_at(&project, &manager, &sql, None).check("clipped range")?;

    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(&cell.column, &"r", "cell.column == \"r\"")?;
    ensure_eq(
        &cell.total,
        &original.trim_end_matches(['\r', '\n']).chars().count(),
        "cell.total == original.trim_end_matches(['\\r', '\\n']).chars().count()",
    )?;
    Ok(())
}

#[test]
fn a_bad_line_range_is_an_invalid_params_error() -> TestResult {
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();

    let error = ask_at(&project, &manager, "SELECT line_range('a', 0, 1)", None)
        .check_err("start 0 is refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure(
        error.message.contains("start must be at least 1"),
        format!("{}", error.message),
    )?;
    Ok(())
}

#[test]
fn a_text_with_a_nul_is_continued_as_bytes_because_sqlite_text_functions_stop_at_it() -> TestResult
{
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();
    let source = "SELECT 'ab' || char(0) || 'cdefghij' AS t";

    let page = ask_at(&project, &manager, source, Some(14)).check("clipped page")?;

    let cell = &page.clipped_cells.first().check("page.clipped_cells[0]")?;
    ensure_eq(
        &(cell.kind, cell.unit, cell.total),
        &("text", "bytes", 11),
        "(cell.kind, cell.unit, cell.total) == (\"text\", \"bytes\", 11)",
    )?;
    let rest = ask_at(
        &project,
        &manager,
        &format!("SELECT hex({}) AS h FROM ({source})", cell.continue_with),
        None,
    )
    .check("continuation")?;
    let expected = "ab\0cdefghij"
        .bytes()
        .skip(cell.returned)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .concat();
    ensure_eq(
        &first_string(&rest, "h")?,
        &expected,
        "first_string(&rest, \"h\")? == expected",
    )?;
    Ok(())
}

#[test]
fn a_duplicate_column_name_is_refused_before_a_row_is_clipped() -> TestResult {
    let project = activated_project(&[])?;
    let manager = SourceSyncManager::default();
    // The projection is refused before any row is read, so a first row that
    // would otherwise have to be clipped never reaches the clipper.
    let sql = "SELECT zeroblob(5000) AS id, zeroblob(5000) AS id";

    let error = ask_at(&project, &manager, sql, Some(100))
        .check_err("the row cannot carry two `id` columns")?;

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
fn both_row_builders_reject_duplicate_column_names() -> TestResult {
    let names = vec!["id".to_owned(), "id".to_owned()];
    let values = [
        rusqlite::types::Value::Integer(1),
        rusqlite::types::Value::Integer(2),
    ];

    let plain = super::value::row_to_json(&names, &values)
        .check_err("the unclipped builder cannot key two columns by one name")?;
    let clipped = super::clip::clip_row(&names, &values, 100, 0)
        .err()
        .check("the clipped builder cannot key two columns by one name")?;

    for error in [plain, clipped] {
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
    }
    Ok(())
}
