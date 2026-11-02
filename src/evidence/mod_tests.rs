use super::*;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn an_unavailable_span_is_normalizer_unavailable_and_warns_with_the_span_error() -> TestResult {
    let (availability, logs) = crate::store::test_capture::capture_warns(|| {
        span_unavailable("binding", &crate::model::SpanError::ReversedBytes)
    })?;

    assert_eq!(
        availability,
        crate::model::SpanAvailability::NormalizerUnavailable
    );
    assert!(
        logs.contains("evidence span failed local range validation"),
        "{logs}"
    );
    assert!(logs.contains("context=\"binding\""), "{logs}");
    assert!(
        logs.contains("span end byte precedes start byte"),
        "the warn keeps the real span error: {logs}"
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
fn a_parser_that_cannot_be_built_is_logged_and_reported_as_none() -> TestResult {
    let (tree, logs) = crate::store::test_capture::capture_warns(|| {
        parse_tree("no-such-language-for-evidence-test", "x", None)
    })?;

    assert_eq!(tree.err(), Some(NoTree::Failed));
    assert!(logs.contains("could not build a parser"), "{logs}");
    Ok(())
}

#[test]
fn a_parse_under_a_spent_deadline_is_a_timeout_and_never_starts() -> TestResult {
    let deadline = crate::util::Deadline::after(std::time::Duration::ZERO);

    let (tree, logs) = crate::store::test_capture::capture_warns(|| {
        parse_tree("no-such-language-for-evidence-test", "x", Some(&deadline))
    })?;

    assert_eq!(tree.err(), Some(NoTree::TimedOut));
    assert!(
        !logs.contains("could not build a parser"),
        "the parser is never built: {logs}"
    );
    Ok(())
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
