use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};

#[test]
fn sampled_content_with_no_detectable_language_is_content_only() {
    let result = analyze("README", Some("plain text"));

    assert_eq!(result.run.availability, ParserAvailability::Unavailable);
    assert_eq!(result.run.outcome, ParseOutcome::NotAttempted);
    assert!(!result.language_detected);
    assert_eq!(
        result.run.completeness,
        ExtractionCompleteness::ContentOnly,
        "the text is indexed, so 'Unavailable' would read as 'no content'"
    );
    assert!(result.run.validate().is_ok());
}

#[test]
fn content_free_binary_text_is_unavailable() {
    let result = analyze("main.rs", None);

    assert_eq!(result.run.availability, ParserAvailability::Unavailable);
    assert_eq!(result.run.completeness, ExtractionCompleteness::Unavailable);
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
fn a_cleanly_parsed_file_reports_full_extraction() {
    let result = analyze("lib.rs", Some("pub fn a() {}\n"));

    assert_eq!(result.run.outcome, ParseOutcome::Succeeded);
    assert_eq!(
        result.run.completeness,
        ExtractionCompleteness::Full,
        "nothing degraded the extraction, so it must not report Partial"
    );
}

#[test]
fn syntax_error_diagnostics_degrade_extraction_to_partial() {
    let result = analyze("lib.rs", Some("fn broken( {\n"));

    assert!(
        matches!(result.run.outcome, ParseOutcome::SyntaxErrors { .. }),
        "expected a degraded parse, got {:?}",
        result.run.outcome
    );
    assert_eq!(
        result.run.completeness,
        ExtractionCompleteness::Partial,
        "an error tree means extraction covered only part of the file"
    );
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

/// Per-file parse logs identify the file by a redacted path id, not by the
/// raw project-relative path, and a pack error's `Display` (which embeds the
/// absolute grammar-library path) must never be formatted into the record.
#[test]
fn a_parser_load_failure_logs_the_redacted_path_not_the_raw_one() {
    let relative = "src/private-name.rs";
    let raw_library_path =
        "/home/private-user/.cache/tree-sitter-language-pack/libtree-sitter-rust.so";
    let error = PackError::DynamicLoad(format!("Failed to load library {raw_library_path}"));
    let (_, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for parser_load");
        load_failed_result(relative, "rust".to_owned(), &error);
    });

    assert!(
        logs.contains("capture liveness control for parser_load"),
        "the capture must see this thread's events, captured: {logs}"
    );
    assert!(logs.contains("parser failed to load"), "got: {logs}");
    assert!(
        logs.contains("operation=\"parser_load\""),
        "the record must name the phase: {logs}"
    );
    assert!(
        logs.contains("error_kind=\"dynamic_load\""),
        "the record must carry the static error category: {logs}"
    );
    assert!(
        logs.contains(&crate::util::redacted_path(std::path::Path::new(relative))),
        "the redacted path id must identify the file: {logs}"
    );
    assert!(
        !logs.contains(relative),
        "the raw project-relative path must not be logged: {logs}"
    );
    assert!(
        !logs.contains(raw_library_path) && !logs.contains("private-user"),
        "the pack's absolute library path must not be logged: {logs}"
    );
}
