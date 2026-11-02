use super::*;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn an_unavailable_span_keeps_the_real_span_error_in_its_reason() -> TestResult {
    let availability = span_unavailable("binding", &crate::model::SpanError::ReversedBytes);

    let crate::model::SpanAvailability::NormalizerUnavailable { reason } = availability else {
        return Err(format!(
            "expected NormalizerUnavailable, got {availability:?}"
        ))
        .check("unexpected outcome");
    };
    assert!(reason.starts_with("binding span"), "{reason}");
    assert!(
        reason.contains("span end byte precedes start byte"),
        "{reason}"
    );
    Ok(())
}

#[test]
fn a_node_range_inside_a_code_point_is_an_error_not_a_panic() -> TestResult {
    let source = "a\u{e9}b";

    assert!(node_text(source, 0, 2).is_err());
    assert!(node_text(source, 0, 99).is_err());
    assert_eq!(node_text(source, 1, 3).check("whole code point")?, "\u{e9}");
    Ok(())
}

#[test]
fn a_parser_that_cannot_be_built_is_logged_and_reported_as_none() {
    let (tree, logs) = crate::store::test_capture::capture_warns(|| {
        parse_tree("no-such-language-for-evidence-test", "x", None)
    });

    assert_eq!(tree.err(), Some(NoTree::Failed));
    assert!(logs.contains("could not build a parser"), "{logs}");
}

#[test]
fn a_parse_under_a_spent_deadline_is_a_timeout_and_never_starts() {
    let deadline = crate::util::Deadline::after(std::time::Duration::ZERO);

    let (tree, logs) = crate::store::test_capture::capture_warns(|| {
        parse_tree("no-such-language-for-evidence-test", "x", Some(&deadline))
    });

    assert_eq!(tree.err(), Some(NoTree::TimedOut));
    assert!(
        !logs.contains("could not build a parser"),
        "the parser is never built: {logs}"
    );
}

#[test]
fn a_parse_with_time_left_returns_its_tree() -> TestResult {
    let deadline = crate::util::Deadline::after(std::time::Duration::from_mins(1));

    let tree = parse_tree("rust", "fn a() {}", Some(&deadline))
        .map_err(|failure| format!("{failure:?}"))
        .check("parse a rust tree")?;

    assert_eq!(tree.root_node().kind(), "source_file");
    Ok(())
}
