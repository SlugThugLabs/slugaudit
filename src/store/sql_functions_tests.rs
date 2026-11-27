use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn connection() -> TestResult<Connection> {
    let connection = Connection::open_in_memory().check("in-memory database")?;
    register(&connection).check("register line functions")?;
    Ok(connection)
}

fn connection_with_cap(cap: usize) -> TestResult<Connection> {
    let connection = Connection::open_in_memory().check("in-memory database")?;
    register_with_cap(&connection, cap).check("register line functions with cap")?;
    Ok(connection)
}

fn range(text: &str, start: i64, end: i64) -> TestResult<String> {
    connection()?
        .query_row("SELECT line_range(?1, ?2, ?3)", (text, start, end), |row| {
            row.get(0)
        })
        .check("line_range")
}

fn count(text: &str) -> TestResult<i64> {
    connection()?
        .query_row("SELECT line_count(?1)", [text], |row| row.get(0))
        .check("line_count")
}

fn error_of(sql: &str) -> TestResult<String> {
    Ok(connection()?
        .query_row(sql, [], |row| row.get::<_, Option<String>>(0))
        .check_err("the call must fail")?
        .to_string())
}

#[test]
fn default_line_function_input_cap_matches_sqlite_max_value_bytes() -> TestResult {
    ensure_eq(
        &u64::try_from(MAX_LINE_FUNCTION_INPUT_BYTES).check("fits u64")?,
        &crate::model::SQLITE_MAX_VALUE_BYTES,
        "line scalar functions accept any storable SQLite TEXT value",
    )?;
    Ok(())
}

#[test]
fn line_count_follows_the_one_based_terminator_model() -> TestResult {
    ensure_eq(&count("")?, &0, "empty string has zero lines")?;
    ensure_eq(&count("a")?, &1, "single unterminated line counts as one")?;
    ensure_eq(
        &count("a\nb")?,
        &2,
        "two lines without trailing newline count as two",
    )?;
    ensure_eq(
        &count("a\nb\n")?,
        &2,
        "trailing newline does not add a phantom empty line",
    )?;
    ensure_eq(&count("\n")?, &1, "lone newline counts as one empty line")?;
    ensure_eq(
        &count("a\n\n")?,
        &2,
        "line followed by blank terminated line counts as two",
    )?;
    ensure_eq(&count("a\r\nb\r\n")?, &2, "CRLF lines count as two")?;
    Ok(())
}

#[test]
fn line_range_returns_inclusive_lines_without_the_last_terminator() -> TestResult {
    let text = "one\ntwo\nthree\nfour\n";

    ensure_eq(
        &range(text, 2, 3)?,
        &"two\nthree",
        "middle inclusive slice returns lines 2 through 3 without trailing newline",
    )?;
    ensure_eq(&range(text, 1, 1)?, &"one", "single first line slice")?;
    ensure_eq(&range(text, 4, 4)?, &"four", "single final line slice")?;
    ensure_eq(
        &range(text, 1, 4)?,
        &"one\ntwo\nthree\nfour",
        "full range slice strips only the final newline",
    )?;
    Ok(())
}

#[test]
fn line_range_past_the_end_stops_at_eof_and_a_start_past_eof_is_empty() -> TestResult {
    ensure_eq(
        &range("a\nb", 2, 99)?,
        &"b",
        "end beyond EOF clamps at EOF on unterminated text",
    )?;
    ensure_eq(
        &range("a\nb\n", 2, i64::MAX)?,
        &"b",
        "i64::MAX end clamps at EOF on newline-terminated text",
    )?;
    ensure_eq(
        &range("a\nb\n", 3, 3)?,
        &"",
        "start past final newline returns empty string",
    )?;
    ensure_eq(
        &range("a\nb", 3, 4)?,
        &"",
        "start past unterminated EOF returns empty string",
    )?;
    ensure_eq(
        &range("", 1, 1)?,
        &"",
        "range on empty text returns empty string",
    )?;
    ensure_eq(
        &range("a", i64::MAX, i64::MAX)?,
        &"",
        "i64::MAX start returns empty string",
    )?;
    Ok(())
}

#[test]
fn line_range_preserves_crlf_between_lines_and_drops_the_last_one() -> TestResult {
    let text = "a\r\nb\r\nc\r\n";

    ensure_eq(
        &range(text, 1, 2)?,
        &"a\r\nb",
        "interior CRLF is preserved while final CRLF is stripped",
    )?;
    ensure_eq(
        &range(text, 3, 3)?,
        &"c",
        "final CRLF line strips CR and LF",
    )?;
    ensure_eq(
        &range("a\nb\r\nc", 1, 3)?,
        &"a\nb\r\nc",
        "mixed LF and CRLF line endings stay intact between lines",
    )?;
    Ok(())
}

#[test]
fn line_range_keeps_empty_lines_and_multibyte_text_intact() -> TestResult {
    let text = "héllo\n\n日本語 🦀\nend";

    ensure_eq(&range(text, 2, 2)?, &"", "blank line returns empty string")?;
    ensure_eq(
        &range(text, 1, 3)?,
        &"héllo\n\n日本語 🦀",
        "multibyte UTF-8 and interior blank lines round-trip intact",
    )?;
    ensure_eq(
        &range(text, 3, 4)?,
        &"日本語 🦀\nend",
        "multibyte line through unterminated EOF round-trips intact",
    )?;
    Ok(())
}

#[test]
fn a_78k_line_file_returns_exactly_lines_40000_to_40200() -> TestResult {
    let lines: Vec<String> = (1..=78_000)
        .map(|number| format!("line {number} ünï 日本語"))
        .collect();
    let text = lines.join("\n");

    let got = range(&text, 40_000, 40_200)?;

    ensure_eq(
        &got,
        &lines
            .get(39_999..40_200)
            .check("lines 39_999..40_200 exist")?
            .join("\n"),
        "slice of 78k-line file matches exact 1-based range",
    )?;
    ensure_eq(&count(&text)?, &78_000, "78k-line file counts 78,000 lines")?;
    Ok(())
}

#[test]
fn null_inputs_give_null() -> TestResult {
    let connection = connection()?;

    let values: (Option<i64>, Option<String>, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT line_count(NULL), line_range(NULL, 1, 2), line_range('a', NULL, 2), \
             line_range('a', 1, NULL)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .check("null calls succeed")?;

    ensure_eq(
        &values,
        &(None, None, None, None),
        "any NULL argument propagates NULL",
    )?;
    Ok(())
}

#[test]
fn invalid_arguments_fail_with_a_clear_message() -> TestResult {
    let start = error_of("SELECT line_range('a', 0, 1)")?;
    let order = error_of("SELECT line_range('a', 3, 2)")?;
    let real = error_of("SELECT line_range('a', 1.5, 2)")?;
    let text_number = error_of("SELECT line_range('a', '1', 2)")?;
    let blob = error_of("SELECT line_count(x'00')")?;

    ensure(start.contains("start must be at least 1"), &start)?;
    ensure(order.contains("must not be less than start"), &order)?;
    ensure(real.contains("start must be an INTEGER"), &real)?;
    ensure(
        text_number.contains("start must be an INTEGER"),
        &text_number,
    )?;
    ensure(blob.contains("must be TEXT"), &blob)?;
    Ok(())
}

/// R7: an input exactly at the function cap is accepted, while one byte over
/// the cap is refused with a static error that carries no part of the input.
#[test]
fn line_function_input_cap_accepts_exact_boundary_and_refuses_one_byte_over() -> TestResult {
    const TEST_CAP: usize = 64 * 1024;
    let conn = connection_with_cap(TEST_CAP)?;
    let at_cap = "a".repeat(TEST_CAP);
    let over_cap = "a".repeat(TEST_CAP + 1);

    let line_total: i64 = conn
        .query_row("SELECT line_count(?1)", [&at_cap], |row| row.get(0))
        .check("at-cap line_count succeeds")?;
    let sliced: String = conn
        .query_row("SELECT line_range(?1, 1, 1)", [&at_cap], |row| row.get(0))
        .check("at-cap line_range succeeds")?;
    ensure_eq(&line_total, &1, "at-cap single line counts as 1")?;
    ensure_eq(
        &sliced.len(),
        &TEST_CAP,
        "at-cap line_range returns full line",
    )?;

    let count_error = conn
        .query_row("SELECT line_count(?1)", [&over_cap], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .check_err("over-cap line_count must fail")?
        .to_string();
    let range_error = conn
        .query_row("SELECT line_range(?1, 1, 1)", [&over_cap], |row| {
            row.get::<_, Option<String>>(0)
        })
        .check_err("over-cap line_range must fail")?
        .to_string();

    ensure(
        count_error.contains("line_count: TEXT argument exceeds"),
        &count_error,
    )?;
    ensure(
        range_error.contains("line_range: TEXT argument exceeds"),
        &range_error,
    )?;
    ensure(
        !count_error.contains("aaaa"),
        format!("the error must not echo the value: {count_error}"),
    )?;
    Ok(())
}

#[test]
fn both_functions_are_registered_as_deterministic() -> TestResult {
    let connection = connection()?;
    connection
        .execute_batch(
            "CREATE TABLE t (body TEXT, n INTEGER GENERATED ALWAYS AS (line_count(body)) STORED)",
        )
        .check("a generated column accepts only deterministic functions")?;
    connection
        .execute_batch("CREATE INDEX i ON t (line_range(body, 1, 1))")
        .check("an index expression accepts only deterministic functions")?;
    Ok(())
}
