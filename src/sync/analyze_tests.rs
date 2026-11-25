use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};
use crate::parse::fetch_gate::test_controls;
use crate::sync::analyze_results::COOLDOWN_REASON;
use crate::sync::analyze_seam::forced_failure;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq, ensure_ne};

/// `analyze` under the default `max_parse_bytes` and a deadline no test
/// parse comes near; shadows the glob import.
fn analyze(relative_path: &str, content: Option<&str>) -> TestResult<ParseResult> {
    super::analyze(
        relative_path,
        content,
        2 * 1024 * 1024,
        std::time::Duration::from_mins(10),
        &generous_deadline(),
    )
    .check("analyze the file")
}

fn generous_deadline() -> Deadline {
    Deadline::after(std::time::Duration::from_mins(10))
}

#[test]
fn sampled_content_with_no_detectable_language_is_content_only() -> TestResult {
    let result = analyze("README", Some("plain text"))?;

    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Unavailable,
        "result.run.availability == ParserAvailability::Unavailable",
    )?;
    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::NotAttempted,
        "result.run.outcome == ParseOutcome::NotAttempted",
    )?;
    ensure(!result.language_detected, "!result.language_detected")?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "the text is indexed, so 'Unavailable' would read as 'no content'",
    )?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn content_free_binary_text_is_unavailable() -> TestResult {
    let result = analyze("main.rs", None)?;

    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Unavailable,
        "result.run.availability == ParserAvailability::Unavailable",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::Unavailable,
        "result.run.completeness == ExtractionCompleteness::Unavailable",
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
    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Available,
        "result.run.availability == ParserAvailability::Available",
    )?;
    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Succeeded,
        "result.run.outcome == ParseOutcome::Succeeded",
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

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Succeeded,
        "result.run.outcome == ParseOutcome::Succeeded",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::Full,
        "nothing degraded the extraction, so it must not report Partial",
    )?;
    Ok(())
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

fn stored_run(connection: &rusqlite::Connection, path: &str) -> TestResult<(String, String)> {
    connection
        .query_row(
            "SELECT parser_availability, parse_outcome FROM files WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .check("stored run")
}

/// A project whose first publish had every grammar download fail, so its
/// files are stored `LoadFailed`, with content untouched afterwards.
fn project_with_failed_download() -> TestResult<(
    tempfile::TempDir,
    tempfile::TempDir,
    rusqlite::Connection,
    String,
)> {
    use crate::progress::NoopProgressSink;
    use crate::sync::test_support::write;

    let project = tempfile::tempdir().check("project dir")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection =
        crate::store::open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    write(project.path(), "a.rs", b"fn a() {}")?;
    let revision = {
        let _forced = forced_failure::force(|| PackError::Download("offline".into()));
        crate::sync::publish::publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
            .check("publish with the download failing")?
            .revision_id
    };
    ensure_eq(
        &stored_run(&connection, "a.rs")?,
        &("LoadFailed".to_owned(), "NotAttempted".to_owned()),
        "stored_run(&connection, \"a.rs\")? == (\"LoadFailed\".to_owned(), \"NotAttempted\".to_owned())",
    )?;
    // The failed fetch closed the gate; these callers test the retry itself.
    test_controls::elapse_cooldown();
    Ok((project, db_dir, connection, revision))
}

#[test]
fn a_load_failed_file_is_reparsed_by_a_full_publish_with_unchanged_content() -> TestResult {
    let (project, _db_dir, mut connection, _revision) = project_with_failed_download()?;

    let report = crate::sync::publish::publish(
        &mut connection,
        project.path(),
        "1.0",
        &crate::progress::NoopProgressSink,
    )
    .check("second publish with loading allowed")?;

    ensure_eq(&report.modified, &1, format!("{report:?}"))?;
    ensure_eq(
        &stored_run(&connection, "a.rs")?,
        &("Available".to_owned(), "Succeeded".to_owned()),
        "stored_run(&connection, \"a.rs\")? == (\"Available\".to_owned(), \"Succeeded\".to_owned())",
    )?;
    Ok(())
}

#[test]
fn a_succeeded_file_with_unchanged_content_is_still_skipped_by_the_next_sync() -> TestResult {
    let (project, _db_dir, mut connection, revision) = crate::sync::test_support::setup_project()?;

    let report = crate::sync::publish::publish(
        &mut connection,
        project.path(),
        "1.0",
        &crate::progress::NoopProgressSink,
    )
    .check("second publish")?;

    ensure_eq(&report.revision_id, &revision, "no new revision")?;
    ensure_eq(
        &(report.added, report.modified),
        &(0, 0),
        "(report.added, report.modified) == (0, 0)",
    )?;
    ensure_eq(&report.unchanged, &2, "report.unchanged == 2")?;
    Ok(())
}

fn stored_reason(connection: &rusqlite::Connection, path: &str) -> TestResult<String> {
    connection
        .query_row(
            "SELECT parse_error_reason FROM files WHERE path = ?1",
            [path],
            |row| row.get(0),
        )
        .check("stored reason")
}

/// A project with two Rust files, synced through the manager while every
/// grammar fetch fails. Returns the manager, the project and the synced state.
fn synced_with_failing_fetch() -> TestResult<(
    crate::sync::SourceSyncManager,
    tempfile::TempDir,
    crate::sync::SyncedProject,
)> {
    use crate::sync::test_support::{create_project, sync_project, write_file};

    let manager = crate::sync::SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "a.rs", b"fn a() {}")?;
    write_file(&project, "b.rs", b"fn b() {}")?;
    let synced = {
        let _forced = forced_failure::force(|| PackError::Download("offline".into()));
        sync_project(&manager, &project)?
    };
    Ok((manager, project, synced))
}

#[test]
fn a_second_file_in_the_same_sync_does_not_reach_the_loader_after_a_fetch_failure() -> TestResult {
    let (_manager, _project, synced) = synced_with_failing_fetch()?;

    ensure_eq(&forced_failure::calls(), &1, "one failed fetch, then none")?;
    let connection = crate::store::open_read_write(&synced.database_path).check("open db")?;
    for path in ["a.rs", "b.rs"] {
        ensure_eq(
            &stored_run(&connection, path)?,
            &("LoadFailed".to_owned(), "NotAttempted".to_owned()),
            path.to_string(),
        )?;
    }
    let reasons = [
        stored_reason(&connection, "a.rs")?,
        stored_reason(&connection, "b.rs")?,
    ];
    ensure_eq(
        &reasons.iter().filter(|r| **r == COOLDOWN_REASON).count(),
        &1,
        format!("exactly the file after the failure carries the cool-down reason: {reasons:?}"),
    )?;
    Ok(())
}

#[test]
fn an_unchanged_load_failed_file_publishes_no_revision_during_the_cooldown() -> TestResult {
    use crate::sync::test_support::sync_project;
    let (manager, project, first) = synced_with_failing_fetch()?;
    let calls_after_first_sync = forced_failure::calls();

    let second = sync_project(&manager, &project)?;

    ensure_eq(&second.revision_id, &first.revision_id, "no new revision")?;
    ensure_eq(
        &forced_failure::calls(),
        &calls_after_first_sync,
        "no loader call",
    )?;
    Ok(())
}

#[test]
fn a_load_failed_file_is_retried_and_parses_once_the_cooldown_has_elapsed() -> TestResult {
    use crate::sync::test_support::sync_project;
    let (manager, project, first) = synced_with_failing_fetch()?;
    test_controls::elapse_cooldown();

    let second = sync_project(&manager, &project)?;

    ensure_ne(
        &second.revision_id,
        &first.revision_id,
        "the retry publishes",
    )?;
    let connection = crate::store::open_read_write(&second.database_path).check("open db")?;
    for path in ["a.rs", "b.rs"] {
        ensure_eq(
            &stored_run(&connection, path)?,
            &("Available".to_owned(), "Succeeded".to_owned()),
            path.to_string(),
        )?;
    }
    Ok(())
}

#[test]
fn a_failed_retry_after_the_cooldown_starts_a_new_cooldown() -> TestResult {
    let _forced = forced_failure::force(|| PackError::Download("offline".into()));
    analyze("a.rs", Some("fn a() {}"))?;
    test_controls::elapse_cooldown();
    ensure(
        crate::parse::fetch_gate::load_failed_retry_due().check("read the fetch gate")?,
        "crate::parse::fetch_gate::load_failed_retry_due().check(\"read the fetch gate\")?",
    )?;

    analyze("a.rs", Some("fn a() {}"))?;

    ensure(
        !crate::parse::fetch_gate::load_failed_retry_due().check("read the fetch gate")?,
        "!crate::parse::fetch_gate::load_failed_retry_due().check(\"read the fetch gate\")?",
    )?;
    ensure_eq(&forced_failure::calls(), &2, "forced_failure::calls() == 2")?;
    Ok(())
}

#[test]
fn a_grammar_already_on_the_host_still_parses_during_the_cooldown() -> TestResult {
    {
        let _forced = forced_failure::force(|| PackError::Download("offline".into()));
        analyze("a.rs", Some("fn a() {}"))?;
    }
    test_controls::set_grammar_cached(true);

    let result = analyze("b.rs", Some("fn b() {}"))?;

    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Available,
        "result.run.availability == ParserAvailability::Available",
    )?;
    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Succeeded,
        "result.run.outcome == ParseOutcome::Succeeded",
    )?;
    Ok(())
}

#[test]
fn a_load_failure_that_is_not_a_fetch_failure_does_not_close_the_gate() -> TestResult {
    let _forced = forced_failure::force(|| PackError::DynamicLoad("bad library".into()));

    let result = analyze("a.rs", Some("fn a() {}"))?;

    ensure(
        matches!(
            result.run.availability,
            ParserAvailability::LoadFailed { .. }
        ),
        "matches!( result.run.availability, ParserAvailability::LoadFailed { .. } )",
    )?;
    ensure(
        crate::parse::fetch_gate::load_failed_retry_due().check("read the fetch gate")?,
        "crate::parse::fetch_gate::load_failed_retry_due().check(\"read the fetch gate\")?",
    )?;
    ensure(
        crate::parse::fetch_gate::is_locally_failed("rust").check("read local load failure")?,
        "non-fetch load failure must record language as locally failed",
    )?;
    Ok(())
}

/// Publishes `files` (path, content) into a fresh project and returns the
/// open database; the project and database directories live as long as it.
fn published(
    files: &[(&str, &str)],
) -> TestResult<(tempfile::TempDir, tempfile::TempDir, rusqlite::Connection)> {
    use crate::progress::NoopProgressSink;
    use crate::sync::test_support::write;

    let project = tempfile::tempdir().check("project dir")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection =
        crate::store::open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    for (path, content) in files {
        write(project.path(), path, content.as_bytes())?;
    }
    crate::sync::publish::publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
        .check("publish")?;
    Ok((project, db_dir, connection))
}

fn count(connection: &rusqlite::Connection, sql: &str) -> TestResult<i64> {
    connection
        .query_row(sql, [], |row| row.get(0))
        .check("count query")
}

#[path = "analyze_prose_tests.rs"]
mod prose;
