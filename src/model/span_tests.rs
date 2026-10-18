use super::*;

#[test]
fn accepts_empty_eof_span() {
    let position = Position { line: 2, column: 4 };
    let span = Span::new(8, 8, position, position).expect("valid span");
    assert_eq!(span.byte_len(), 0);
}

#[test]
fn rejects_reversed_ranges() {
    let position = Position { line: 0, column: 0 };
    assert_eq!(
        Span::new(2, 1, position, position),
        Err(SpanError::ReversedBytes)
    );
}

#[test]
fn a_span_whose_end_position_precedes_its_start_is_rejected() {
    let start = Position { line: 3, column: 5 };
    let end = Position { line: 3, column: 4 };

    let result = Span::new(10, 12, start, end);

    assert_eq!(result, Err(SpanError::ReversedPosition));
}

#[test]
fn a_mid_codepoint_offset_floors_to_the_character_boundary() {
    // `é` occupies bytes 0..2, so byte 1 is inside it: the column must be
    // measured from the boundary before it rather than panicking.
    let source = "é=1";
    assert_eq!(char_column(source, 1), 0);
    assert_eq!(char_column(source, 2), 1);
}

#[test]
fn an_offset_past_the_end_measures_the_whole_line() {
    let source = "a\nbc";
    assert_eq!(char_column(source, 99), 2);
}

/// The pre-index implementations, kept verbatim as the oracle.
fn legacy_position(source: &str, byte: usize) -> Position {
    let clamped = byte.min(source.len());
    let mut floor = clamped;
    while !source.is_char_boundary(floor) {
        floor -= 1;
    }
    Position {
        line: saturating_u32(source[..floor].bytes().filter(|b| *b == b'\n').count()),
        column: char_column(source, byte),
    }
}

fn assert_index_matches_legacy(source: &str) {
    let index = LineIndex::new(source);
    for offset in 0..=source.len() + 2 {
        let expected = legacy_position(source, offset);
        assert_eq!(index.position(offset), expected, "position at {offset}");
        assert_eq!(index.column(offset), expected.column, "column at {offset}");
        assert_eq!(index.line(offset), expected.line, "line at {offset}");
    }
}

#[test]
fn the_line_index_matches_the_legacy_computation_on_varied_inputs() {
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
        assert_index_matches_legacy(input);
    }
}

#[test]
fn the_line_index_matches_the_legacy_computation_on_generated_text() {
    let alphabet = ['a', 'b', '\n', '\r', 'é', '日', '\u{1F600}', ' '];
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut text = String::new();
    for _ in 0..3000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let pick = usize::try_from(state >> 33).expect("fits") % alphabet.len();
        text.push(alphabet[pick]);
    }

    assert_index_matches_legacy(&text);
}

#[test]
fn a_multi_megabyte_single_line_answers_every_column_query_quickly() {
    // 6 MB on one line, mostly multibyte. The per-call `char_column`
    // walk costs ~3e6 steps per query; 200k queries is ~6e11 steps.
    let text = "aé日".repeat(1_000_000);
    let index = LineIndex::new(&text);
    let started = std::time::Instant::now();

    let mut last = 0;
    for offset in (0..text.len()).step_by(text.len() / 200_000) {
        last = index.column(offset);
    }

    assert!(last > 0);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "200k column queries took {:?}",
        started.elapsed()
    );
}
