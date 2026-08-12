//! Unit tests for `check_source_limits`.

use super::counter::{code_lines, exception_reason};

#[test]
fn empty_string_is_zero() {
    assert_eq!(code_lines(""), 0);
}

#[test]
fn only_whitespace_is_zero() {
    assert_eq!(code_lines("   \n\t\n  \n"), 0);
}

#[test]
fn line_comment_only_is_zero() {
    assert_eq!(code_lines("// nothing\n// here\n"), 0);
}

#[test]
fn block_comment_only_is_zero() {
    assert_eq!(code_lines("/* this\nspans\nlines */\n"), 0);
}

#[test]
fn nested_block_comment_does_not_close_early() {
    // Depth-2 block comment: the inner `*/` must NOT close the outer
    // comment.
    let src = "/* outer /* inner */ still in outer */\n";
    assert_eq!(code_lines(src), 0);
}

#[test]
fn fn_definition_counts_one() {
    assert_eq!(code_lines("fn foo() {}\n"), 1);
}

#[test]
fn code_then_blank_lines_still_one() {
    assert_eq!(code_lines("\n\nfn x() {}\n\n\n"), 1);
}

#[test]
fn two_separate_code_lines() {
    assert_eq!(code_lines("fn a() {}\nfn b() {}\n"), 2);
}

#[test]
fn string_does_not_split_line() {
    // The string content has an escaped newline (\n), but Rust
    // considers it a single line of code (no actual newline byte
    // present).
    assert_eq!(code_lines("let x = \"a\\nb\";\n"), 1);
}

#[test]
fn raw_string_with_embedded_newline_counts_each_physical_line() {
    // A raw string spanning multiple physical lines via an embedded
    // newline byte counts each physical line of code — each line has
    // at least one token, so each counts. This mirrors the prior
    // Python implementation's behavior: finalize-on-newline runs at
    // the top of the loop regardless of state, so multi-line raw
    // strings count per physical line.
    assert_eq!(code_lines("let x = r#\"a\nb\"#;\n"), 2);
}

#[test]
fn char_literal_counts() {
    assert_eq!(code_lines("let c = 'a';\n"), 1);
}

#[test]
fn escaped_char_literal_counts() {
    assert_eq!(code_lines("let c = '\\n';\n"), 1);
}

#[test]
fn unterminated_char_literal_falls_through() {
    // `'` alone is not a valid char literal; the apostrophe is just a
    // stray byte that should still count as code.
    assert_eq!(code_lines("let c = ';\n"), 1);
}

#[test]
fn code_after_end_of_line_comment_counts() {
    assert_eq!(code_lines("// hdr\nfn x() {}\n"), 1);
}

#[test]
fn code_after_block_comment_counts() {
    assert_eq!(code_lines("/* hdr */\nfn x() {}\n"), 1);
}

#[test]
fn exception_reason_parsed() {
    let src = "// slugaudit-line-exception: approved-by=agent; reason=foo bar\nfn x() {}\n";
    assert_eq!(exception_reason(src).as_deref(), Some("foo bar"));
}

#[test]
fn exception_reason_absent_returns_none() {
    assert_eq!(exception_reason("fn x() {}\n"), None);
}

#[test]
fn exception_wrong_approver_returns_none() {
    let src = "// slugaudit-line-exception: approved-by=human; reason=foo\nfn x() {}\n";
    assert_eq!(exception_reason(src), None);
}
