//! Unit tests for `check_source_limits`.

use super::approval::{Exception, reason as exception_reason};
use super::counter::{PRODUCTION_CEILING, TEST_FILE_CEILING, Verdict, code_lines, verdict};
use super::{FileOutcome, check_file};

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
    // at least one token, so each counts. Finalize-on-newline
    // runs at the top of the loop regardless of state, so multi-line raw
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
fn non_ascii_code_is_scanned_without_slicing_inside_a_char() {
    let source = "let tree = \"├──\"; let x = 'é'; // ├\nfn r() {}\n";

    let lines = code_lines(source);

    assert_eq!(
        lines, 2,
        "multi-byte chars must not split or panic the scan"
    );
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
    let src = "// slugaudit-line-exception: approved-by=slugthug; reason=foo bar\nfn x() {}\n";
    assert_eq!(
        exception_reason(src)
            .map(|exception| exception.reason)
            .as_deref(),
        Some("foo bar")
    );
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

#[test]
fn dated_human_exception_is_parsed() {
    let src =
        "// slugaudit-line-exception: approved-by=human-user; date=2026-08-28; reason=approved\n";
    assert_eq!(
        exception_reason(src)
            .map(|exception| exception.reason)
            .as_deref(),
        Some("[human-approved 2026-08-28] approved")
    );
}

// --- line-limit policy (verdict) ---

#[test]
fn test_files_auto_pass_up_to_the_500_ceiling() {
    assert_eq!(verdict(0, None, true), Verdict::Pass);
    assert_eq!(
        verdict(300, Some(Exception::agent("reason".to_owned())), true),
        Verdict::Pass
    );
    assert_eq!(verdict(500, None, true), Verdict::Pass);
}

#[test]
fn test_files_over_500_hard_fail_even_with_an_exception() {
    assert_eq!(
        verdict(501, Some(Exception::agent("reason".to_owned())), true),
        Verdict::FailHard {
            ceiling: TEST_FILE_CEILING
        }
    );
}

#[test]
fn production_files_below_the_exception_floor_auto_pass() {
    assert_eq!(verdict(0, None, false), Verdict::Pass);
    assert_eq!(verdict(199, None, false), Verdict::Pass);
}

#[test]
fn production_files_need_an_exception_from_200_to_300() {
    assert_eq!(verdict(200, None, false), Verdict::FailNeedsException);
    assert_eq!(verdict(300, None, false), Verdict::FailNeedsException);
    assert_eq!(
        verdict(250, Some(Exception::agent("contract".to_owned())), false),
        Verdict::PassWithException {
            reason: "contract".into()
        }
    );
}

#[test]
fn production_files_over_300_hard_fail_even_with_an_exception() {
    assert_eq!(
        verdict(301, Some(Exception::agent("reason".to_owned())), false),
        Verdict::FailHard {
            ceiling: PRODUCTION_CEILING
        }
    );
}

const HUMAN_ANNOTATION: &str =
    "// slugaudit-line-exception: approved-by=human-user; date=2026-10-18; reason=cohesive\n";
const AGENT_ANNOTATION: &str =
    "// slugaudit-line-exception: approved-by=slugthug; reason=cohesive\n";

#[test]
fn a_dated_human_exception_passes_a_production_file_at_301_lines() {
    let exception = exception_reason(HUMAN_ANNOTATION).expect("human annotation parses");

    assert_eq!(
        verdict(301, Some(exception.clone()), false),
        Verdict::PassWithException {
            reason: exception.reason
        }
    );
}

#[test]
fn a_dated_human_exception_passes_a_production_file_at_350_lines() {
    let exception = exception_reason(HUMAN_ANNOTATION).expect("human annotation parses");

    assert_eq!(
        verdict(350, Some(exception.clone()), false),
        Verdict::PassWithException {
            reason: exception.reason
        }
    );
}

#[test]
fn an_agent_exception_fails_hard_at_301_and_350_lines() {
    let exception = exception_reason(AGENT_ANNOTATION).expect("agent annotation parses");
    let expected = Verdict::FailHard {
        ceiling: PRODUCTION_CEILING,
    };

    assert_eq!(verdict(301, Some(exception.clone()), false), expected);
    assert_eq!(verdict(350, Some(exception), false), expected);
}

#[test]
fn an_agent_reason_imitating_the_human_display_prefix_fails_hard() {
    // Admission must key on the parsed approver, not the *reason text*: an
    // agent could otherwise write the reserved display prefix itself and buy
    // the 301–350 band.
    let annotation = "// slugaudit-line-exception: approved-by=slugthug; \
                      reason=[human-approved 2026-10-19] pretend\n";
    let exception = exception_reason(annotation).expect("agent annotation parses");

    assert_eq!(
        verdict(301, Some(exception.clone()), false),
        Verdict::FailHard {
            ceiling: PRODUCTION_CEILING
        }
    );
    assert_eq!(
        verdict(350, Some(exception), false),
        Verdict::FailHard {
            ceiling: PRODUCTION_CEILING
        }
    );
}

#[test]
fn a_dated_human_exception_does_not_rescue_351_lines() {
    let exception = exception_reason(HUMAN_ANNOTATION).expect("human annotation parses");

    assert_eq!(
        verdict(351, Some(exception), false),
        Verdict::FailHard {
            ceiling: PRODUCTION_CEILING
        }
    );
}

#[test]
fn check_file_reports_a_pass_line_with_a_root_relative_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("src").join("ok.rs");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, "fn a() {}\n").expect("write");
    assert_eq!(
        check_file(dir.path(), &path),
        FileOutcome::Line("source-limit: src/ok.rs: 1 code lines; pass".to_string())
    );
}

#[test]
fn check_file_fails_a_file_over_the_hard_ceiling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.rs");
    let source = "fn a() {}\n".repeat(PRODUCTION_CEILING + 1);
    std::fs::write(&path, source).expect("write");
    assert_eq!(
        check_file(dir.path(), &path),
        FileOutcome::Failure(format!(
            "big.rs: {} code lines (>{PRODUCTION_CEILING}; hard failure)",
            PRODUCTION_CEILING + 1
        ))
    );
}

#[test]
fn check_file_reports_a_read_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("missing.rs");
    let FileOutcome::Failure(message) = check_file(dir.path(), &path) else {
        panic!("a missing file must fail");
    };
    assert!(
        message.starts_with("missing.rs: read error: "),
        "unexpected message: {message}"
    );
}

#[test]
fn check_file_fails_loudly_when_the_path_is_outside_the_root() {
    let root = tempfile::tempdir().expect("tempdir");
    let other = tempfile::tempdir().expect("tempdir");
    let path = other.path().join("x.rs");
    std::fs::write(&path, "fn a() {}\n").expect("write");
    assert_eq!(
        check_file(root.path(), &path),
        FileOutcome::Failure(format!(
            "{}: path is not under the project root {}",
            path.display(),
            root.path().display()
        ))
    );
}
