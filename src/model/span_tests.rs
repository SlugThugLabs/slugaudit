use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn accepts_empty_eof_span() -> TestResult {
    let position = Position { line: 2, column: 4 };
    let span = Span::new(8, 8, position, position).check("valid span")?;
    ensure_eq(&span.byte_len(), &Some(0), "span.byte_len() == Some(0)")?;
    Ok(())
}

#[test]
fn rejects_reversed_ranges() -> TestResult {
    let position = Position { line: 0, column: 0 };
    ensure_eq(
        &Span::new(2, 1, position, position),
        &Err(SpanError::ReversedBytes),
        "Span::new(2, 1, position, position) == Err(SpanError::ReversedBytes)",
    )?;
    Ok(())
}

#[test]
fn a_span_whose_end_position_precedes_its_start_is_rejected() -> TestResult {
    let start = Position { line: 3, column: 5 };
    let end = Position { line: 3, column: 4 };

    let result = Span::new(10, 12, start, end);

    ensure_eq(
        &result,
        &Err(SpanError::ReversedPosition),
        "result == Err(SpanError::ReversedPosition)",
    )?;
    Ok(())
}

#[test]
fn a_mid_codepoint_offset_floors_to_the_character_boundary() -> TestResult {
    // `é` occupies bytes 0..2, so byte 1 is inside it: the column must be
    // measured from the boundary before it rather than panicking.
    let source = "é=1";
    ensure_eq(
        &char_column(source, 1).check("column inside é")?,
        &0,
        "char_column(source, 1).check(\"column inside é\")? == 0",
    )?;
    ensure_eq(
        &char_column(source, 2).check("column after é")?,
        &1,
        "char_column(source, 2).check(\"column after é\")? == 1",
    )?;
    Ok(())
}

#[test]
fn an_offset_past_the_end_measures_the_whole_line() -> TestResult {
    let source = "a\nbc";
    ensure_eq(
        &char_column(source, 99).check("column past the end")?,
        &2,
        "char_column(source, 99).check(\"column past the end\")? == 2",
    )?;
    Ok(())
}

#[test]
fn a_position_that_fits_u32_converts_exactly() -> TestResult {
    ensure_eq(
        &position_u32(0).check("zero")?,
        &0,
        "position_u32(0).check(\"zero\")? == 0",
    )?;
    ensure_eq(
        &position_u32(u32::MAX as usize).check("the largest position")?,
        &u32::MAX,
        "position_u32(u32::MAX as usize).check(\"the largest position\")? == u32::MAX",
    )?;
    Ok(())
}

#[test]
fn a_position_past_u32_max_is_an_error_naming_the_value() -> TestResult {
    let too_far = u32::MAX as usize + 1;

    let error = position_u32(too_far).check_err("a position past u32::MAX")?;

    ensure_eq(
        &error.to_string(),
        &format!("source position {too_far} exceeds u32::MAX"),
        "error.to_string() == format!(\"source position {too_far} exceeds u32::MAX\")",
    )?;
    Ok(())
}

/// The per-call implementations (no index), kept verbatim as the oracle.
fn legacy_position(source: &str, byte: usize) -> TestResult<Position> {
    let clamped = byte.min(source.len());
    let floor = (0..=clamped)
        .rev()
        .find(|candidate| source.is_char_boundary(*candidate))
        .check("a char boundary at or below the offset")?;
    Ok(Position {
        line: position_u32(source[..floor].bytes().filter(|b| *b == b'\n').count())
            .check("legacy line")?,
        column: char_column(source, byte).check("legacy column")?,
    })
}

fn assert_index_matches_legacy(source: &str) -> TestResult {
    let index = LineIndex::new(source).check("index source")?;
    let end = source
        .len()
        .checked_add(2)
        .check("the source length plus two fits usize")?;
    for offset in 0..=end {
        let expected = legacy_position(source, offset)?;
        ensure_eq(
            &index.position(offset).check("indexed position")?,
            &expected,
            format!("position at {offset}"),
        )?;
        ensure_eq(
            &index.column(offset).check("indexed column")?,
            &expected.column,
            format!("column at {offset}"),
        )?;
        ensure_eq(
            &index.line(offset).check("indexed line")?,
            &expected.line,
            format!("line at {offset}"),
        )?;
    }
    Ok(())
}

#[test]
fn the_line_index_matches_the_legacy_computation_on_varied_inputs() -> TestResult {
    let long_multibyte = format!(
        "{}\nx{}\r\n{}",
        "é日".repeat(400),
        "\u{1F600}".repeat(150),
        "a".repeat(700)
    );
    let inputs = [
        "",
        "\n",
        "abc",
        "abc\n",
        "\n\n\n",
        "a\r\nb\r\n\r\nc",
        "é=1\n日本語\nx",
        "\u{1F600}\u{1F600}\n\u{1F600}",
        "no trailing newline é",
        long_multibyte.as_str(),
    ];

    for input in inputs {
        assert_index_matches_legacy(input)?;
    }
    Ok(())
}

#[test]
fn the_line_index_matches_the_legacy_computation_on_generated_text() -> TestResult {
    let alphabet = ['a', 'b', '\n', '\r', 'é', '日', '\u{1F600}', ' '];
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut text = String::new();
    for _ in 0..3000 {
        // KEEP wrapping: a 64-bit linear congruential generator wraps by definition
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let pick = usize::try_from(state >> 33).check("fits")? % alphabet.len();
        text.push(
            *alphabet
                .get(pick)
                .check("pick is below the alphabet length")?,
        );
    }

    assert_index_matches_legacy(&text)?;
    Ok(())
}

#[test]
fn a_multi_megabyte_single_line_answers_every_column_query_quickly() -> TestResult {
    // 6 MB on one line, mostly multibyte. The per-call `char_column`
    // walk costs ~3e6 steps per query; 200k queries is ~6e11 steps.
    let text = "aé日".repeat(1_000_000);
    let index = LineIndex::new(&text).check("index source")?;
    let started = std::time::Instant::now();

    let mut last = 0;
    for offset in (0..text.len()).step_by(text.len() / 200_000) {
        last = index.column(offset).check("column query")?;
    }

    ensure(last > 0, "last > 0")?;
    ensure(
        started.elapsed() < std::time::Duration::from_secs(5),
        format!("200k column queries took {:?}", started.elapsed()),
    )?;
    Ok(())
}

#[test]
fn a_reversed_deserialized_span_has_no_byte_length() -> TestResult {
    let json = r#"{"start_byte":9,"end_byte":3,"start":{"line":0,"column":0},"end":{"line":0,"column":0}}"#;

    let span: Span = serde_json::from_str(json).check("deserialize bypasses Span::new")?;

    ensure_eq(&span.byte_len(), &None, "span.byte_len() == None")?;
    Ok(())
}
