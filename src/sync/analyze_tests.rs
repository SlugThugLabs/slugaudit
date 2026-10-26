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

#[test]
fn a_grammar_download_error_is_recorded_as_load_failed() {
    let _forced = forced_failure::force(|| PackError::Download("offline".into()));

    let result = analyze("lib.rs", Some("pub fn a() {}"));

    assert!(
        matches!(
            result.run.availability,
            ParserAvailability::LoadFailed { .. }
        ),
        "got {:?}",
        result.run
    );
    assert_eq!(result.run.outcome, ParseOutcome::NotAttempted);
    assert_eq!(result.run.completeness, ExtractionCompleteness::ContentOnly);
    assert_eq!(result.language.as_deref(), Some("rust"));
    assert!(result.run.validate().is_ok());
}

#[test]
fn every_grammar_fetch_error_variant_is_a_load_failure() {
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
        assert!(is_load_failure(error), "{error:?} must be a load failure");
    }
}

#[test]
fn an_unrelated_parse_error_stays_failed_not_load_failed() {
    let _forced = forced_failure::force(|| PackError::ParseFailed);

    let result = analyze("lib.rs", Some("pub fn a() {}"));

    assert_eq!(result.run.availability, ParserAvailability::Available);
    assert!(
        matches!(result.run.outcome, ParseOutcome::Failed { .. }),
        "got {:?}",
        result.run
    );
    assert!(result.run.validate().is_ok());
}

fn stored_run(connection: &rusqlite::Connection, path: &str) -> (String, String) {
    connection
        .query_row(
            "SELECT parser_availability, parse_outcome FROM files WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("stored run")
}

/// A project whose first publish had every grammar download fail, so its
/// files are stored `LoadFailed`, with content untouched afterwards.
fn project_with_failed_download() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    rusqlite::Connection,
    String,
) {
    use crate::progress::NoopProgressSink;
    use crate::sync::test_support::write;

    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection =
        crate::store::open_read_write(&db_dir.path().join("project.db")).expect("open db");
    write(project.path(), "a.rs", b"fn a() {}");
    let revision = {
        let _forced = forced_failure::force(|| PackError::Download("offline".into()));
        crate::sync::publish::publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
            .expect("publish with the download failing")
            .revision_id
    };
    assert_eq!(
        stored_run(&connection, "a.rs"),
        ("LoadFailed".to_owned(), "NotAttempted".to_owned())
    );
    (project, db_dir, connection, revision)
}

#[test]
fn a_load_failed_file_is_reparsed_by_reconcile_with_unchanged_content() {
    let (project, _db_dir, mut connection, revision) = project_with_failed_download();

    let report = crate::sync::reconcile::reconcile_dirty_paths(
        &mut connection,
        project.path(),
        ["a.rs".to_owned()].into(),
        std::collections::HashSet::new(),
        Some(&revision),
    )
    .expect("reconcile with loading allowed");

    assert_eq!(report.reconciled, 1, "{report:?}");
    assert_eq!(report.unchanged, 0);
    assert_eq!(
        stored_run(&connection, "a.rs"),
        ("Available".to_owned(), "Succeeded".to_owned())
    );
}

#[test]
fn a_load_failed_file_is_a_sweep_candidate_even_when_its_stat_is_unchanged() {
    let (project, _db_dir, connection, _revision) = project_with_failed_download();
    let deadline = crate::util::Deadline::after(std::time::Duration::from_secs(60));

    let report = crate::sync::sweep::sweep(&connection, project.path(), &deadline).expect("sweep");

    assert!(
        report.candidates.contains("a.rs"),
        "{:?}",
        report.candidates
    );
    assert_eq!(report.unchanged, 0);
}

#[test]
fn a_load_failed_file_is_reparsed_by_a_full_publish_with_unchanged_content() {
    let (project, _db_dir, mut connection, _revision) = project_with_failed_download();

    let report = crate::sync::publish::publish(
        &mut connection,
        project.path(),
        "1.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("second publish with loading allowed");

    assert_eq!(report.modified, 1, "{report:?}");
    assert_eq!(
        stored_run(&connection, "a.rs"),
        ("Available".to_owned(), "Succeeded".to_owned())
    );
}

#[test]
fn a_succeeded_file_with_unchanged_content_is_still_skipped_by_reconcile() {
    let (project, _db_dir, mut connection, revision) = crate::sync::test_support::setup_project();

    let report = crate::sync::reconcile::reconcile_dirty_paths(
        &mut connection,
        project.path(),
        ["a.rs".to_owned()].into(),
        std::collections::HashSet::new(),
        Some(&revision),
    )
    .expect("reconcile");

    assert_eq!((report.reconciled, report.unchanged), (0, 1));
}
