use super::*;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn parser_resource_and_parse_outcome_are_independent() -> TestResult {
    let run = ParserRun {
        availability: ParserAvailability::Available,
        outcome: ParseOutcome::SyntaxErrors { count: 1 },
        completeness: ExtractionCompleteness::Partial,
    };
    run.validate().check("`run.validate()` is ok")?;
    Ok(())
}

#[test]
fn unavailable_parser_cannot_look_successful() -> TestResult {
    let run = ParserRun {
        availability: ParserAvailability::Unavailable,
        outcome: ParseOutcome::Succeeded,
        completeness: ExtractionCompleteness::Full,
    };
    ensure_eq(
        &run.validate(),
        &Err("an unavailable parser cannot report a parse outcome"),
        "run.validate() == Err(\"an unavailable parser cannot report a parse outcome\")",
    )?;
    Ok(())
}

#[test]
fn successful_parsing_cannot_claim_unavailable_extraction() -> TestResult {
    let run = ParserRun {
        availability: ParserAvailability::Available,
        outcome: ParseOutcome::Succeeded,
        completeness: ExtractionCompleteness::Unavailable,
    };

    ensure_eq(
        &run.validate(),
        &Err("successful parsing cannot have unavailable extraction"),
        "run.validate() == Err(\"successful parsing cannot have unavailable extraction\")",
    )?;
    Ok(())
}

#[test]
fn sql_text_ignores_variant_payloads() -> TestResult {
    ensure_eq(
        &ParserAvailability::LoadFailed { reason: "x".into() }.as_sql_text(),
        &"LoadFailed",
        "ParserAvailability::LoadFailed { reason: \"x\".into() }.as_sql_text() == \"LoadFailed\"",
    )?;
    ensure_eq(
        &ParseOutcome::Failed { reason: "x".into() }.as_sql_text(),
        &"Failed",
        "ParseOutcome::Failed { reason: \"x\".into() }.as_sql_text() == \"Failed\"",
    )?;
    Ok(())
}

#[test]
fn error_reason_prefers_load_failure_over_parse_failure() -> TestResult {
    let run = ParserRun {
        availability: ParserAvailability::LoadFailed {
            reason: "load".into(),
        },
        outcome: ParseOutcome::NotAttempted,
        completeness: ExtractionCompleteness::Unavailable,
    };
    ensure_eq(
        &run.error_reason().as_deref(),
        &Some("load"),
        "run.error_reason().as_deref() == Some(\"load\")",
    )?;
    Ok(())
}

#[test]
fn a_withheld_parse_stores_as_not_attempted_with_its_reason_code() -> TestResult {
    let run = ParserRun {
        availability: ParserAvailability::Unavailable,
        outcome: ParseOutcome::Withheld {
            reason: ParseOutcome::OVERSIZE_FOR_PARSE.to_owned(),
        },
        completeness: ExtractionCompleteness::ContentOnly,
    };

    ensure_eq(&run.validate(), &Ok(()), "run.validate() == Ok(())")?;
    ensure_eq(
        &run.outcome.as_sql_text(),
        &"NotAttempted",
        "run.outcome.as_sql_text() == \"NotAttempted\"",
    )?;
    ensure_eq(
        &run.error_reason().as_deref(),
        &Some("oversize_for_parse"),
        "run.error_reason().as_deref() == Some(\"oversize_for_parse\")",
    )?;
    Ok(())
}

#[test]
fn error_reason_is_none_when_nothing_failed() -> TestResult {
    let run = ParserRun {
        availability: ParserAvailability::Available,
        outcome: ParseOutcome::Succeeded,
        completeness: ExtractionCompleteness::Partial,
    };
    ensure_eq(&run.error_reason(), &None, "run.error_reason() == None")?;
    Ok(())
}
