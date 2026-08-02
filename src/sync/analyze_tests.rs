use super::*;
use crate::model::{ParseOutcome, ParserAvailability};

#[test]
fn a_file_with_no_detectable_language_is_marked_unavailable() {
    let result = analyze("README", Some("plain text"));
    assert_eq!(result.run.availability, ParserAvailability::Unavailable);
    assert_eq!(result.run.outcome, ParseOutcome::NotAttempted);
    assert!(!result.language_detected);
    assert!(result.run.validate().is_ok());
}

#[test]
fn binary_content_is_never_analyzed() {
    let result = analyze("main.rs", None);
    assert_eq!(result.run.availability, ParserAvailability::Unavailable);
    assert!(result.evidence.is_empty());
    assert!(result.run.validate().is_ok());
}

#[test]
fn a_supported_language_produces_real_evidence() {
    let result = analyze("lib.rs", Some("pub fn a() {}"));
    assert_eq!(result.language.as_deref(), Some("rust"));
    assert_eq!(result.run.availability, ParserAvailability::Available);
    assert_eq!(result.run.outcome, ParseOutcome::Succeeded);
    assert!(!result.evidence.is_empty());
    assert!(result.run.validate().is_ok());
}

#[test]
fn malformed_source_reports_syntax_errors_with_a_real_count() {
    let result = analyze("lib.rs", Some("fn broken( {\n"));
    match result.run.outcome {
        ParseOutcome::SyntaxErrors { count } => assert!(count > 0),
        other => panic!("expected SyntaxErrors, got {other:?}"),
    }
    assert!(result.run.validate().is_ok());
}

#[test]
fn every_produced_run_satisfies_its_own_invariant() {
    // The whole point of routing through ParserRun instead of raw strings:
    // every path this function can take must be a state validate() accepts.
    for result in [
        analyze("README", Some("plain text")),
        analyze("main.rs", None),
        analyze("lib.rs", Some("pub fn a() {}")),
        analyze("lib.rs", Some("fn broken( {\n")),
    ] {
        assert!(result.run.validate().is_ok(), "{:?}", result.run);
    }
}

#[test]
fn load_failure_classification_is_disjoint_from_parse_failure() {
    let load_failure = PackError::LanguageNotFound("x".into());
    let parse_failure = PackError::ParseFailed;
    assert!(is_load_failure(&load_failure));
    assert!(!is_load_failure(&parse_failure));
}
