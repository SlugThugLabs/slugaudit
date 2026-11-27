//! Findings: refused inputs and each write or read failure surfacing as a typed error.
use super::tests::base_request;
use super::*;
use crate::tools::MAX_PATH_BYTES;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn validation_error(request: FindingRequest) -> TestResult<ErrorData> {
    match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Err(error) => Ok(error),
        Ok(_) => Err("the request was accepted".to_owned()).check("the request must be refused"),
    }
}

#[test]
fn input_caps_refuse_an_oversized_path_file_and_zero_line() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;

    let mut path = base_request(&project, "lib.rs");
    path.path = "x".repeat(MAX_PATH_BYTES + 1);
    ensure_eq(
        &validation_error(path)?.message,
        &"path exceeds 32768 bytes",
        "validation_error(path)?.message == \"path exceeds 32768 bytes\"",
    )?;

    let mut file = base_request(&project, "lib.rs");
    file.file = "x".repeat(MAX_FILE_BYTES + 1);
    ensure_eq(
        &validation_error(file)?.message,
        &"file exceeds 8192 bytes",
        "validation_error(file)?.message == \"file exceeds 8192 bytes\"",
    )?;

    let mut line = base_request(&project, "lib.rs");
    line.line_start = 0;
    ensure_eq(
        &validation_error(line)?.message,
        &"line_start and line_end are one-based; zero is not a valid line",
        "validation_error(line)?.message == \"line_start and line_end are one-based; zero is not a valid line\"",
    )?;
    Ok(())
}

/// Runs `insert_finding` against a hand-written schema so one statement can
/// fail on its own, without a production test hook: one `findings` column the
/// statement names is absent (schema drift).
fn insert_finding_error(
    request: &FindingRequest,
    schema: &str,
    seed: Option<&str>,
) -> TestResult<ErrorData> {
    let mut connection = Connection::open_in_memory().check("in-memory database")?;
    connection
        .execute_batch(schema)
        .check("create test schema")?;
    if let Some(seed) = seed {
        connection
            .execute(seed, [])
            .check("seed the stored finding")?;
    }
    let tx = connection.transaction().check("transaction")?;
    insert_finding(&tx, request, "hash".to_owned(), "rev").check_err("the statement must fail")
}

/// Each fallible statement in `insert_finding` — the restatement lookup, the
/// restatement UPDATE, the `created_at_unix` read, and the INSERT — must
/// surface a database failure as an internal error, never as a bad input.
#[test]
fn each_findings_write_statement_failure_is_an_internal_error() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    let request = base_request(&project, "lib.rs");
    let cases: &[(&str, &str, Option<&str>, &str)] = &[
        (
            "restatement lookup",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, title TEXT, description TEXT, \
             created_at_unix INTEGER, evidence_revision TEXT);",
            None,
            "no such column: category",
        ),
        (
            "restatement update",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, category TEXT, title TEXT, description TEXT, \
             created_at_unix INTEGER, evidence_revision TEXT);",
            Some(
                "INSERT INTO findings (path, source_hash, line_start, line_end, category, title, \
                 description, created_at_unix, evidence_revision) \
                 VALUES ('lib.rs', 'hash', 1, 1, 'correctness', 'Reviewed conclusion', 'old', 1, \
                 'rev')",
            ),
            "no such column: severity",
        ),
        (
            "created_at read",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, severity TEXT, category TEXT, title TEXT, \
             description TEXT, evidence_revision TEXT);",
            None,
            "no such column: created_at_unix",
        ),
        (
            "finding insert",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, severity TEXT, category TEXT, title TEXT, \
             description TEXT, created_at_unix INTEGER);",
            None,
            "evidence_revision",
        ),
    ];

    for &(label, schema, seed, expected) in cases {
        let error = insert_finding_error(&request, schema, seed)?;
        ensure_eq(
            &error.code,
            &rmcp::model::ErrorCode::INTERNAL_ERROR,
            "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
        )?;
        ensure(
            error.message.contains(expected),
            format!("{label}: {}", error.message),
        )?;
    }
    Ok(())
}

/// A database that cannot be read is not the caller's `file` argument being
/// wrong: it must be an internal error that does not blame the input.
#[test]
fn a_failed_files_read_is_an_internal_error_not_a_bad_input() -> TestResult {
    let mut connection = Connection::open_in_memory().check("in-memory database")?;
    let tx = connection.transaction().check("transaction")?;

    let error = fetch_file(&tx, "lib.rs").check_err("a missing files table must fail")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure(
        error.message.contains("no such table: files"),
        format!("{}", error.message),
    )?;
    ensure(
        !error.message.contains("lib.rs"),
        format!("{}", error.message),
    )?;
    Ok(())
}
