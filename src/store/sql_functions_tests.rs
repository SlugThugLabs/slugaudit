use super::*;

fn connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database");
    register(&connection).expect("register line functions");
    connection
}

fn range(text: &str, start: i64, end: i64) -> String {
    connection()
        .query_row("SELECT line_range(?1, ?2, ?3)", (text, start, end), |row| {
            row.get(0)
        })
        .expect("line_range")
}

fn count(text: &str) -> i64 {
    connection()
        .query_row("SELECT line_count(?1)", [text], |row| row.get(0))
        .expect("line_count")
}

fn error_of(sql: &str) -> String {
    connection()
        .query_row(sql, [], |row| row.get::<_, Option<String>>(0))
        .expect_err("the call must fail")
        .to_string()
}

fn error_of_query(sql: &str, text: &str) -> String {
    connection()
        .query_row(sql, [text], |row| row.get::<_, Option<String>>(0))
        .expect_err("the call must fail")
        .to_string()
}

#[test]
fn line_count_follows_the_one_based_terminator_model() {
    assert_eq!(count(""), 0);
    assert_eq!(count("a"), 1);
    assert_eq!(count("a\nb"), 2);
    assert_eq!(count("a\nb\n"), 2);
    assert_eq!(count("\n"), 1);
    assert_eq!(count("a\n\n"), 2);
    assert_eq!(count("a\r\nb\r\n"), 2);
}

#[test]
fn line_range_returns_inclusive_lines_without_the_last_terminator() {
    let text = "one\ntwo\nthree\nfour\n";

    assert_eq!(range(text, 2, 3), "two\nthree");
    assert_eq!(range(text, 1, 1), "one");
    assert_eq!(range(text, 4, 4), "four");
    assert_eq!(range(text, 1, 4), "one\ntwo\nthree\nfour");
}

#[test]
fn line_range_past_the_end_stops_at_eof_and_a_start_past_eof_is_empty() {
    assert_eq!(range("a\nb", 2, 99), "b");
    assert_eq!(range("a\nb\n", 2, i64::MAX), "b");
    assert_eq!(range("a\nb\n", 3, 3), "");
    assert_eq!(range("a\nb", 3, 4), "");
    assert_eq!(range("", 1, 1), "");
    assert_eq!(range("a", i64::MAX, i64::MAX), "");
}

#[test]
fn line_range_preserves_crlf_between_lines_and_drops_the_last_one() {
    let text = "a\r\nb\r\nc\r\n";

    assert_eq!(range(text, 1, 2), "a\r\nb");
    assert_eq!(range(text, 3, 3), "c");
    assert_eq!(range("a\nb\r\nc", 1, 3), "a\nb\r\nc");
}

#[test]
fn line_range_keeps_empty_lines_and_multibyte_text_intact() {
    let text = "héllo\n\n日本語 🦀\nend";

    assert_eq!(range(text, 2, 2), "");
    assert_eq!(range(text, 1, 3), "héllo\n\n日本語 🦀");
    assert_eq!(range(text, 3, 4), "日本語 🦀\nend");
}

#[test]
fn a_78k_line_file_returns_exactly_lines_40000_to_40200() {
    let lines: Vec<String> = (1..=78_000)
        .map(|number| format!("line {number} ünï 日本語"))
        .collect();
    let text = lines.join("\n");

    let got = range(&text, 40_000, 40_200);

    assert_eq!(got, lines[39_999..40_200].join("\n"));
    assert_eq!(count(&text), 78_000);
}

#[test]
fn null_inputs_give_null() {
    let connection = connection();

    let values: (Option<i64>, Option<String>, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT line_count(NULL), line_range(NULL, 1, 2), line_range('a', NULL, 2), \
             line_range('a', 1, NULL)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("null calls succeed");

    assert_eq!(values, (None, None, None, None));
}

#[test]
fn invalid_arguments_fail_with_a_clear_message() {
    let start = error_of("SELECT line_range('a', 0, 1)");
    let order = error_of("SELECT line_range('a', 3, 2)");
    let real = error_of("SELECT line_range('a', 1.5, 2)");
    let text_number = error_of("SELECT line_range('a', '1', 2)");
    let blob = error_of("SELECT line_count(x'00')");

    assert!(start.contains("start must be at least 1"), "{start}");
    assert!(order.contains("must not be less than start"), "{order}");
    assert!(real.contains("start must be an INTEGER"), "{real}");
    assert!(
        text_number.contains("start must be an INTEGER"),
        "{text_number}"
    );
    assert!(blob.contains("must be TEXT"), "{blob}");
}

/// R7: an input exactly at the function cap is still accepted, so the cap
/// bounds work without rejecting the largest value a conservative query
/// budget can return.
#[test]
fn a_line_function_input_at_the_byte_cap_is_accepted() {
    let text = "a".repeat(MAX_LINE_FUNCTION_INPUT_BYTES);

    assert_eq!(count(&text), 1);
    assert_eq!(range(&text, 1, 1).len(), MAX_LINE_FUNCTION_INPUT_BYTES);
}

/// R7: a value one byte over the cap is refused with a static error before
/// any scan or slice copy, and the error carries no part of the value.
#[test]
fn a_line_function_input_over_the_byte_cap_is_refused_with_a_static_error() {
    let text = "a".repeat(MAX_LINE_FUNCTION_INPUT_BYTES + 1);

    let count_error = error_of_query("SELECT line_count(?1)", &text);
    let range_error = error_of_query("SELECT line_range(?1, 1, 1)", &text);

    assert!(
        count_error.contains("line_count: TEXT argument exceeds"),
        "{count_error}"
    );
    assert!(
        range_error.contains("line_range: TEXT argument exceeds"),
        "{range_error}"
    );
    assert!(
        !count_error.contains("aaaa"),
        "the error must not echo the value: {count_error}"
    );
}

#[test]
fn both_functions_are_registered_as_deterministic() {
    let connection = connection();
    connection
        .execute_batch(
            "CREATE TABLE t (body TEXT, n INTEGER GENERATED ALWAYS AS (line_count(body)) STORED)",
        )
        .expect("a generated column accepts only deterministic functions");
    connection
        .execute_batch("CREATE INDEX i ON t (line_range(body, 1, 1))")
        .expect("an index expression accepts only deterministic functions");
}
