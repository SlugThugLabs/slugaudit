//! Tests for the `Failed` outcome constructors.

use super::*;
use slugaudit_test_check::{CheckErr, TestResult, ensure, ensure_eq};

/// A position past `u32::MAX`, as the extractor reports it.
fn position_overflow() -> TestResult<ExtractError> {
    let overflow = crate::model::position_u32(usize::MAX)
        .check_err("usize::MAX does not fit a u32 position")?;
    Ok(ExtractError::Position(overflow))
}

#[test]
fn a_position_overflow_has_the_position_overflow_kind() -> TestResult {
    ensure_eq(
        &extract_error_kind(&position_overflow()?),
        &"position_overflow",
        "extract_error_kind(&position_overflow()?) == \"position_overflow\"",
    )?;
    Ok(())
}

#[test]
fn a_position_overflow_records_failed_with_its_cause_and_logs_its_kind() -> TestResult {
    let error = position_overflow()?;

    let (result, logs) = crate::store::test_capture::capture_at_level(
        || parse_failed_result("src/lib.rs", "rust".to_owned(), &error),
        tracing::Level::DEBUG,
    )?;

    ensure_eq(
        &result.language.as_deref(),
        &Some("rust"),
        "result.language.as_deref() == Some(\"rust\")",
    )?;
    ensure(result.language_detected, "result.language_detected")?;
    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Available,
        "result.run.availability == ParserAvailability::Available",
    )?;
    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: error.to_string(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: error.to_string() }",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "result.run.completeness == ExtractionCompleteness::ContentOnly",
    )?;
    ensure(result.evidence.is_empty(), "result.evidence.is_empty()")?;
    ensure_eq(
        &result.run.validate(),
        &Ok(()),
        "result.run.validate() == Ok(())",
    )?;
    ensure(
        logs.contains("error_kind=\"position_overflow\"") && logs.contains("path=src/lib.rs"),
        format!("the failure must be logged with its stable kind: {logs}"),
    )?;
    Ok(())
}

#[test]
fn a_failed_result_is_a_valid_content_only_run() -> TestResult {
    let result = failed_result("rust".to_owned(), "probe reason".to_owned());

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: "probe reason".to_owned(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: \"probe reason\".to_owned() }",
    )?;
    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Available,
        "result.run.availability == ParserAvailability::Available",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "result.run.completeness == ExtractionCompleteness::ContentOnly",
    )?;
    ensure_eq(
        &result.run.validate(),
        &Ok(()),
        "result.run.validate() == Ok(())",
    )?;
    Ok(())
}
