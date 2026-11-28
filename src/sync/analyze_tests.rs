use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun};
use crate::parse::fetch_gate::test_controls::{self, GateResetGuard};
use crate::parse::local_load_failures::{LocalLoadFailuresGuard, is_locally_failed};
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

pub(super) struct ParseTestGuards {
    _gate: GateResetGuard,
    _failures: LocalLoadFailuresGuard,
}

pub(super) fn reset_parse_guards() -> ParseTestGuards {
    ParseTestGuards {
        _gate: GateResetGuard::new(),
        _failures: LocalLoadFailuresGuard::clear(),
    }
}

pub(super) fn stored_run(connection: &Connection, path: &str) -> TestResult<(String, String)> {
    connection
        .query_row(
            "SELECT parser_availability, parse_outcome FROM files WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .check("stored run")
}

/// `analyze` under the default `max_parse_bytes` and a deadline no test
/// parse comes near; shadows the glob import.
fn analyze(relative_path: &str, content: Option<&str>) -> TestResult<ParseResult> {
    super::analyze(
        relative_path,
        content,
        2 * 1024 * 1024,
        std::time::Duration::from_mins(10),
        &Deadline::after(std::time::Duration::from_mins(10)),
    )
    .check("analyze the file")
}

#[track_caller]
fn assert_run(
    result: &ParseResult,
    availability: ParserAvailability,
    outcome: ParseOutcome,
    completeness: ExtractionCompleteness,
) -> TestResult {
    ensure_eq(
        &result.run,
        &ParserRun {
            availability,
            outcome,
            completeness,
        },
        "result.run",
    )
}

fn prime_fetch_cooldown() -> TestResult<ParseTestGuards> {
    let guards = reset_parse_guards();
    test_controls::set_grammar_cached(false);
    let _forced = forced_failure::force(|| PackError::Download("offline".into()));
    analyze("a.rs", Some("fn a() {}"))?;
    Ok(guards)
}

#[test]
fn sampled_content_with_no_detectable_language_is_content_only() -> TestResult {
    let result = analyze("README", Some("plain text"))?;

    // The text is indexed, so 'Unavailable' would read as 'no content'.
    assert_run(
        &result,
        ParserAvailability::Unavailable,
        ParseOutcome::NotAttempted,
        ExtractionCompleteness::ContentOnly,
    )?;
    ensure(!result.language_detected, "!result.language_detected")?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn content_free_binary_text_is_unavailable() -> TestResult {
    let result = analyze("main.rs", None)?;

    assert_run(
        &result,
        ParserAvailability::Unavailable,
        ParseOutcome::NotAttempted,
        ExtractionCompleteness::Unavailable,
    )?;
    ensure(result.evidence.is_empty(), "result.evidence.is_empty()")?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn a_supported_language_produces_real_evidence() -> TestResult {
    let result = analyze("lib.rs", Some("pub fn a() {}"))?;
    ensure_eq(
        &result.language.as_deref(),
        &Some("rust"),
        "result.language.as_deref() == Some(\"rust\")",
    )?;
    assert_run(
        &result,
        ParserAvailability::Available,
        ParseOutcome::Succeeded,
        ExtractionCompleteness::Full,
    )?;
    ensure(!result.evidence.is_empty(), "!result.evidence.is_empty()")?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn malformed_source_reports_syntax_errors_with_a_real_count() -> TestResult {
    let result = analyze("lib.rs", Some("fn broken( {\n"))?;
    match result.run.outcome {
        ParseOutcome::SyntaxErrors { count } => ensure(count > 0, "count > 0")?,
        other => {
            return Err(format!("expected SyntaxErrors, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn a_cleanly_parsed_file_reports_full_extraction() -> TestResult {
    let result = analyze("lib.rs", Some("pub fn a() {}\n"))?;

    // Nothing degraded the extraction, so it must not report Partial.
    assert_run(
        &result,
        ParserAvailability::Available,
        ParseOutcome::Succeeded,
        ExtractionCompleteness::Full,
    )
}

#[test]
fn syntax_error_diagnostics_degrade_extraction_to_partial() -> TestResult {
    let result = analyze("lib.rs", Some("fn broken( {\n"))?;

    ensure(
        matches!(result.run.outcome, ParseOutcome::SyntaxErrors { .. }),
        format!("expected a degraded parse, got {:?}", result.run.outcome),
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::Partial,
        "an error tree means extraction covered only part of the file",
    )?;
    Ok(())
}

#[test]
fn every_produced_run_satisfies_its_own_invariant() -> TestResult {
    // The whole point of routing through ParserRun instead of raw strings:
    // every path this function can take must be a state validate() accepts.
    for result in [
        analyze("README", Some("plain text"))?,
        analyze("main.rs", None)?,
        analyze("lib.rs", Some("pub fn a() {}"))?,
        analyze("lib.rs", Some("fn broken( {\n"))?,
    ] {
        result.run.validate().check("the parser run validates")?;
    }
    Ok(())
}

#[test]
fn load_failure_classification_is_disjoint_from_parse_failure() -> TestResult {
    let load_failure = PackError::LanguageNotFound("x".into());
    let parse_failure = PackError::ParseFailed;
    ensure(
        is_load_failure(&load_failure),
        "is_load_failure(&load_failure)",
    )?;
    ensure(
        !is_load_failure(&parse_failure),
        "!is_load_failure(&parse_failure)",
    )?;
    Ok(())
}

/// Per-file parse logs name the project-relative path, and a pack error's
/// `Display` (which embeds the absolute grammar-library path) must never be
/// formatted into the record.
#[test]
fn a_parser_load_failure_logs_the_relative_path_not_the_library_path() -> TestResult {
    let relative = "src/private-name.rs";
    let raw_library_path =
        "/home/private-user/.cache/tree-sitter-language-pack/libtree-sitter-rust.so";
    let error = PackError::DynamicLoad(format!("Failed to load library {raw_library_path}"));
    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for parser_load");
        load_failed_result(relative, "rust".to_owned(), &error);
    })?;

    ensure(
        logs.contains("capture liveness control for parser_load"),
        format!("the capture must see this thread's events, captured: {logs}"),
    )?;
    ensure(
        logs.contains("parser failed to load"),
        format!("got: {logs}"),
    )?;
    ensure(
        logs.contains("operation=\"parser_load\""),
        format!("the record must name the phase: {logs}"),
    )?;
    ensure(
        logs.contains("error_kind=\"dynamic_load\""),
        format!("the record must carry the static error category: {logs}"),
    )?;
    ensure(
        logs.contains(&format!("path={relative}")),
        format!("the relative path must identify the file: {logs}"),
    )?;
    ensure(
        !logs.contains(raw_library_path) && !logs.contains("private-user"),
        format!("the pack's absolute library path must not be logged: {logs}"),
    )?;
    Ok(())
}

#[test]
fn a_grammar_download_error_is_recorded_as_load_failed() -> TestResult {
    let _guards = reset_parse_guards();
    let _forced = forced_failure::force(|| PackError::Download("offline".into()));

    let result = analyze("lib.rs", Some("pub fn a() {}"))?;

    ensure(
        matches!(
            result.run.availability,
            ParserAvailability::LoadFailed { .. }
        ),
        format!("got {:?}", result.run),
    )?;
    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::NotAttempted,
        "result.run.outcome == ParseOutcome::NotAttempted",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "result.run.completeness == ExtractionCompleteness::ContentOnly",
    )?;
    ensure_eq(
        &result.language.as_deref(),
        &Some("rust"),
        "result.language.as_deref() == Some(\"rust\")",
    )?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn every_grammar_fetch_error_variant_is_a_load_failure() -> TestResult {
    let fetch_errors = [
        PackError::Download("x".into()),
        PackError::ChecksumMismatch {
            file: "f".into(),
            expected: "a".into(),
            actual: "b".into(),
        },
        PackError::CacheLock("x".into()),
        PackError::Io(std::io::Error::other("x")),
    ];
    for error in &fetch_errors {
        ensure(
            is_load_failure(error),
            format!("{error:?} must be a load failure"),
        )?;
    }
    Ok(())
}

#[test]
fn an_unrelated_parse_error_stays_failed_not_load_failed() -> TestResult {
    let _guards = reset_parse_guards();
    let _forced = forced_failure::force(|| PackError::ParseFailed);

    let result = analyze("lib.rs", Some("pub fn a() {}"))?;

    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Available,
        "result.run.availability == ParserAvailability::Available",
    )?;
    ensure(
        matches!(result.run.outcome, ParseOutcome::Failed { .. }),
        format!("got {:?}", result.run),
    )?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[track_caller]
fn retry_due() -> TestResult<bool> {
    crate::parse::fetch_gate::load_failed_retry_due().check("read the fetch gate")
}

#[test]
fn a_failed_retry_after_the_cooldown_starts_a_new_cooldown() -> TestResult {
    let _guards = prime_fetch_cooldown()?;
    test_controls::elapse_cooldown();
    ensure(retry_due()?, "retry_due()?")?;

    let _forced = forced_failure::force(|| PackError::Download("offline".into()));
    analyze("a.rs", Some("fn a() {}"))?;

    ensure(!retry_due()?, "!retry_due()?")?;
    ensure_eq(&forced_failure::calls(), &2, "forced_failure::calls() == 2")?;
    Ok(())
}

#[test]
fn a_grammar_already_on_the_host_still_parses_during_the_cooldown() -> TestResult {
    let _guards = prime_fetch_cooldown()?;
    test_controls::set_grammar_cached(true);

    let result = analyze("b.rs", Some("fn b() {}"))?;

    assert_run(
        &result,
        ParserAvailability::Available,
        ParseOutcome::Succeeded,
        ExtractionCompleteness::Full,
    )
}

#[test]
fn a_load_failure_that_is_not_a_fetch_failure_does_not_close_the_gate() -> TestResult {
    let _guards = reset_parse_guards();
    let _forced = forced_failure::force(|| PackError::DynamicLoad("bad library".into()));

    let result = analyze("a.rs", Some("fn a() {}"))?;

    ensure(
        matches!(
            result.run.availability,
            ParserAvailability::LoadFailed { .. }
        ),
        "matches!( result.run.availability, ParserAvailability::LoadFailed { .. } )",
    )?;
    ensure(retry_due()?, "retry_due()?")?;
    ensure(
        is_locally_failed("rust").check("read local load failure")?,
        "non-fetch load failure must record language as locally failed",
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "analyze_sync_tests.rs"]
mod sync_tests;

#[path = "analyze_prose_tests.rs"]
mod prose;
