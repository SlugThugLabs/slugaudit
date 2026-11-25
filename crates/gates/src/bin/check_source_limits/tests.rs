//! Unit tests for `check_source_limits`.

use super::approval::{Exception, reason as exception_reason};
use super::counter::{
    PRODUCTION_EXCEPTION_FLOOR, TEST_FILE_EXCEPTION_FLOOR, Verdict, code_lines, verdict,
};
use super::{FileOutcome, check_file};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn empty_string_is_zero() -> TestResult {
    ensure_eq(&code_lines(""), &0, "code_lines(\"\") == 0")?;
    Ok(())
}

#[test]
fn only_whitespace_is_zero() -> TestResult {
    ensure_eq(
        &code_lines("   \n\t\n  \n"),
        &0,
        "code_lines(\" \\n\\t\\n \\n\") == 0",
    )?;
    Ok(())
}

#[test]
fn line_comment_only_is_zero() -> TestResult {
    ensure_eq(
        &code_lines("// nothing\n// here\n"),
        &0,
        "code_lines(\"// nothing\\n// here\\n\") == 0",
    )?;
    Ok(())
}

#[test]
fn block_comment_only_is_zero() -> TestResult {
    ensure_eq(
        &code_lines("/* this\nspans\nlines */\n"),
        &0,
        "code_lines(\"/* this\\nspans\\nlines */\\n\") == 0",
    )?;
    Ok(())
}

#[test]
fn nested_block_comment_does_not_close_early() -> TestResult {
    // Depth-2 block comment: the inner `*/` must NOT close the outer
    // comment.
    let src = "/* outer /* inner */ still in outer */\n";
    ensure_eq(&code_lines(src), &0, "code_lines(src) == 0")?;
    Ok(())
}

#[test]
fn fn_definition_counts_one() -> TestResult {
    ensure_eq(
        &code_lines("fn foo() {}\n"),
        &1,
        "code_lines(\"fn foo() {}\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn code_then_blank_lines_still_one() -> TestResult {
    ensure_eq(
        &code_lines("\n\nfn x() {}\n\n\n"),
        &1,
        "code_lines(\"\\n\\nfn x() {}\\n\\n\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn two_separate_code_lines() -> TestResult {
    ensure_eq(
        &code_lines("fn a() {}\nfn b() {}\n"),
        &2,
        "code_lines(\"fn a() {}\\nfn b() {}\\n\") == 2",
    )?;
    Ok(())
}

#[test]
fn string_does_not_split_line() -> TestResult {
    // The string content has an escaped newline (\n), but Rust
    // considers it a single line of code (no actual newline byte
    // present).
    ensure_eq(
        &code_lines("let x = \"a\\nb\";\n"),
        &1,
        "code_lines(\"let x = \\\"a\\\\nb\\\";\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn raw_string_with_embedded_newline_counts_each_physical_line() -> TestResult {
    // A raw string spanning multiple physical lines via an embedded
    // newline byte counts each physical line of code — each line has
    // at least one token, so each counts. Finalize-on-newline
    // runs at the top of the loop regardless of state, so multi-line raw
    // strings count per physical line.
    ensure_eq(
        &code_lines("let x = r#\"a\nb\"#;\n"),
        &2,
        "code_lines(\"let x = r#\\\"a\\nb\\\"#;\\n\") == 2",
    )?;
    Ok(())
}

#[test]
fn a_string_continuation_ends_its_physical_line() -> TestResult {
    // `\` before the newline continues the string onto the next line, but the
    // two physical lines both hold tokens.
    ensure_eq(
        &code_lines("const S: &str = \"a\\\n    b\";\n"),
        &2,
        "code_lines(\"const S: &str = \\\"a\\\\\\n b\\\";\\n\") == 2",
    )?;
    Ok(())
}

#[test]
fn an_escaped_newline_in_a_char_or_string_counts_each_line_once() -> TestResult {
    ensure_eq(
        &code_lines("let a = \"x\\\ny\\\nz\";\nlet b = 1;\n"),
        &4,
        "code_lines(\"let a = \\\"x\\\\\\ny\\\\\\nz\\\";\\nlet b = 1;\\n\") == 4",
    )?;
    ensure_eq(
        &code_lines("let a = \"x\\\n\";\n"),
        &2,
        "code_lines(\"let a = \\\"x\\\\\\n\\\";\\n\") == 2",
    )?;
    Ok(())
}

#[test]
fn char_literal_counts() -> TestResult {
    ensure_eq(
        &code_lines("let c = 'a';\n"),
        &1,
        "code_lines(\"let c = 'a';\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn escaped_char_literal_counts() -> TestResult {
    ensure_eq(
        &code_lines("let c = '\\n';\n"),
        &1,
        "code_lines(\"let c = '\\\\n';\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn unterminated_char_literal_falls_through() -> TestResult {
    // `'` alone is not a valid char literal; the apostrophe is just a
    // stray byte that should still count as code.
    ensure_eq(
        &code_lines("let c = ';\n"),
        &1,
        "code_lines(\"let c = ';\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn non_ascii_code_is_scanned_without_slicing_inside_a_char() -> TestResult {
    let source = "let tree = \"├──\"; let x = 'é'; // ├\nfn r() {}\n";

    let lines = code_lines(source);

    ensure_eq(
        &lines,
        &2,
        "multi-byte chars must not split or panic the scan",
    )?;
    Ok(())
}

#[test]
fn code_after_end_of_line_comment_counts() -> TestResult {
    ensure_eq(
        &code_lines("// hdr\nfn x() {}\n"),
        &1,
        "code_lines(\"// hdr\\nfn x() {}\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn code_after_block_comment_counts() -> TestResult {
    ensure_eq(
        &code_lines("/* hdr */\nfn x() {}\n"),
        &1,
        "code_lines(\"/* hdr */\\nfn x() {}\\n\") == 1",
    )?;
    Ok(())
}

#[test]
fn exception_reason_parsed() -> TestResult {
    let src = "// slugaudit-line-exception: approved-by=slugthuglabs; reason=foo bar\nfn x() {}\n";
    ensure_eq(
        &exception_reason(src)
            .map(|exception| exception.reason)
            .as_deref(),
        &Some("foo bar"),
        "exception_reason(src) .map(|exception| exception.reason) .as_deref() == Some(\"foo bar\")",
    )?;
    Ok(())
}

#[test]
fn exception_reason_absent_returns_none() -> TestResult {
    ensure_eq(
        &exception_reason("fn x() {}\n"),
        &None,
        "exception_reason(\"fn x() {}\\n\") == None",
    )?;
    Ok(())
}

#[test]
fn exception_wrong_approver_returns_none() -> TestResult {
    let src = "// slugaudit-line-exception: approved-by=human; reason=foo\nfn x() {}\n";
    ensure_eq(
        &exception_reason(src),
        &None,
        "exception_reason(src) == None",
    )?;
    Ok(())
}

#[test]
fn a_malformed_annotation_does_not_end_the_scan() -> TestResult {
    let valid = "// slugaudit-line-exception: approved-by=slugthuglabs; reason=later\n";
    for malformed in [
        "// slugaudit-line-exception: approved-by=slugthuglabs; reason=\n",
        "// slugaudit-line-exception: approved-by=slugthuglabs; why=none\n",
        "// slugaudit-line-exception: approved-by=human-user; date=2026-13x; reason=r\n",
        "// slugaudit-line-exception: approved-by=human-user; no-date\n",
        "// slugaudit-line-exception: approved-by=human-user; date=2026-11-02\n",
        "// slugaudit-line-exception: approved-by=human-user date 2026-11-02\n",
    ] {
        let src = format!("{malformed}{valid}fn x() {{}}\n");

        ensure_eq(
            &exception_reason(&src),
            &Some(Exception::approved("later".to_owned())),
            malformed,
        )?;
    }
    Ok(())
}

#[test]
fn dated_human_exception_is_parsed() -> TestResult {
    let src =
        "// slugaudit-line-exception: approved-by=human-user; date=2026-08-28; reason=approved\n";
    ensure_eq(
        &exception_reason(src)
            .map(|exception| exception.reason)
            .as_deref(),
        &Some("[human-approved 2026-08-28] approved"),
        "exception_reason(src) .map(|exception| exception.reason) .as_deref() == Some(\"[human-approved 2026-08-28] approved\")",
    )?;
    Ok(())
}

#[test]
fn split_judge_exception_is_parsed() -> TestResult {
    let src = "// slugaudit-line-exception: approved-by=split-judge; reason=single cohesive job\n";
    ensure_eq(
        &exception_reason(src)
            .map(|exception| exception.reason)
            .as_deref(),
        &Some("single cohesive job"),
        "split-judge exception parses",
    )?;
    Ok(())
}

// --- line-limit policy (verdict) ---

#[test]
fn test_files_auto_pass_up_to_500_and_allow_exceptions_above() -> TestResult {
    ensure_eq(
        &verdict(0, None, true),
        &Verdict::Pass,
        "verdict(0, None, true) == Verdict::Pass",
    )?;
    ensure_eq(
        &verdict(500, None, true),
        &Verdict::Pass,
        "verdict(500, None, true) == Verdict::Pass",
    )?;
    ensure_eq(
        &verdict(501, None, true),
        &Verdict::FailNeedsException {
            floor: TEST_FILE_EXCEPTION_FLOOR,
        },
        "test file > 500 without exception needs one",
    )?;
    ensure_eq(
        &verdict(501, Some(Exception::approved("reason".to_owned())), true),
        &Verdict::PassWithException {
            reason: "reason".into(),
        },
        "test file > 500 with exception passes",
    )?;
    Ok(())
}

#[test]
fn production_files_below_the_exception_floor_auto_pass() -> TestResult {
    ensure_eq(
        &verdict(0, None, false),
        &Verdict::Pass,
        "verdict(0, None, false) == Verdict::Pass",
    )?;
    ensure_eq(
        &verdict(199, None, false),
        &Verdict::Pass,
        "verdict(199, None, false) == Verdict::Pass",
    )?;
    Ok(())
}

#[test]
fn production_files_at_or_above_200_need_an_exception_with_no_hard_cap() -> TestResult {
    ensure_eq(
        &verdict(200, None, false),
        &Verdict::FailNeedsException {
            floor: PRODUCTION_EXCEPTION_FLOOR,
        },
        "200 lines without exception fails",
    )?;
    ensure_eq(
        &verdict(600, None, false),
        &Verdict::FailNeedsException {
            floor: PRODUCTION_EXCEPTION_FLOOR,
        },
        "600 lines without exception fails",
    )?;
    ensure_eq(
        &verdict(250, Some(Exception::approved("contract".to_owned())), false),
        &Verdict::PassWithException {
            reason: "contract".into(),
        },
        "250 lines with exception passes",
    )?;
    ensure_eq(
        &verdict(600, Some(Exception::approved("cohesive".to_owned())), false),
        &Verdict::PassWithException {
            reason: "cohesive".into(),
        },
        "600 lines with exception passes (no hard cap)",
    )?;
    Ok(())
}

#[test]
fn check_file_reports_a_pass_line_with_a_root_relative_path() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("src").join("ok.rs");
    std::fs::create_dir_all(path.parent().check("parent")?).check("mkdir")?;
    std::fs::write(&path, "fn a() {}\n").check("write")?;
    ensure_eq(
        &check_file(dir.path(), &path),
        &FileOutcome::Line("source-limit: src/ok.rs: 1 code lines; pass".to_string()),
        "check_file(dir.path(), &path) == FileOutcome::Line(\"source-limit: src/ok.rs: 1 code lines; pass\".to_string())",
    )?;
    Ok(())
}

#[test]
fn check_file_fails_a_file_at_or_above_floor_without_exception() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("big.rs");
    let source = "fn a() {}\n".repeat(PRODUCTION_EXCEPTION_FLOOR);
    std::fs::write(&path, source).check("write")?;
    ensure_eq(
        &check_file(dir.path(), &path),
        &FileOutcome::Failure(format!(
            "big.rs: {PRODUCTION_EXCEPTION_FLOOR} code lines (>= {PRODUCTION_EXCEPTION_FLOOR} requires a split-judge review / approved exception)"
        )),
        "file at floor without exception fails",
    )?;
    Ok(())
}

#[test]
fn check_file_reports_a_read_error() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("missing.rs");
    let FileOutcome::Failure(message) = check_file(dir.path(), &path) else {
        return Err("a missing file must fail".to_owned()).check("unexpected outcome");
    };
    ensure(
        message.starts_with("missing.rs: read error: "),
        format!("unexpected message: {message}"),
    )?;
    Ok(())
}

#[test]
fn check_file_fails_loudly_when_the_path_is_outside_the_root() -> TestResult {
    let root = tempfile::tempdir().check("tempdir")?;
    let other = tempfile::tempdir().check("tempdir")?;
    let path = other.path().join("x.rs");
    std::fs::write(&path, "fn a() {}\n").check("write")?;
    ensure_eq(
        &check_file(root.path(), &path),
        &FileOutcome::Failure(format!(
            "{}: path is not under the project root {}",
            path.display(),
            root.path().display()
        )),
        "check_file(root.path(), &path) == FileOutcome::Failure(format!( \"{}: path is not under the project root {}\", path.display(), root.path().display() ))",
    )?;
    Ok(())
}
