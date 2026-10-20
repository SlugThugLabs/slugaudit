use super::*;
use crate::sync;
use crate::tools::test_support::activated_project;
use crate::util::SESSION_TEST_LOCK;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use std::fs;

fn base_request(project: &tempfile::TempDir, file: &str) -> FindingRequest {
    FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: file.to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "medium".to_owned(),
        category: "correctness".to_owned(),
        title: "Reviewed conclusion".to_owned(),
        description: "A conclusion the AI actually checked.".to_owned(),
    }
}

fn finding_status(connection: &Connection, id: i64) -> String {
    connection
        .query_row("SELECT status FROM findings WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("read finding status")
}

#[test]
fn persists_exactly_the_supplied_conclusion() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let response = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("finding succeeds");

    assert_eq!(response.0.status, "current");
    assert!(!response.0.source_hash.is_empty());
    assert!(response.0.id > 0);
}

#[test]
fn a_modified_file_invalidates_its_finding_on_next_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let response = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("finding succeeds");
    let id = response.0.id;

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("resync");

    assert_eq!(finding_status(&connection, id), "stale");
}

#[test]
fn a_deleted_file_invalidates_its_finding_on_next_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let response = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("finding succeeds");
    let id = response.0.id;

    fs::remove_file(project.path().join("lib.rs")).expect("delete file");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("resync");

    assert_eq!(finding_status(&connection, id), "stale");
}

#[test]
fn an_untouched_file_keeps_its_finding_current() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    fs::write(project.path().join("other.rs"), b"pub fn b() {}\n").expect("write second file");
    let response = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("finding succeeds");
    let id = response.0.id;

    fs::write(
        project.path().join("other.rs"),
        b"pub fn b() { changed(); }\n",
    )
    .expect("modify other file");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("resync");

    assert_eq!(finding_status(&connection, id), "current");
}

#[test]
fn sync_never_creates_a_finding_on_its_own() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("sync");

    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count findings");
    assert_eq!(count, 0);
}

#[test]
fn empty_title_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = base_request(&project, "lib.rs");
    request.title = String::new();
    let err = match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Ok(_) => panic!("empty title must error"),
        Err(err) => err,
    };
    assert!(err.message.contains("title"));
}

#[test]
fn reversed_line_range_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 10;
    request.line_end = 1;
    let err = match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Ok(_) => panic!("reversed line range must error"),
        Err(err) => err,
    };
    assert!(err.message.contains("line_end"));
}

#[test]
fn a_finding_against_an_unindexed_file_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = base_request(&project, "does_not_exist.rs");
    let err = match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Ok(_) => panic!("unindexed file must error"),
        Err(err) => err,
    };
    assert_eq!(
        err.message,
        "does_not_exist.rs is not indexed in the current revision"
    );
    let data = err.data.expect("an unindexed file carries a payload");
    assert_eq!(data["code"], "file_not_indexed");
}

#[test]
fn a_line_number_past_the_files_real_length_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 1;
    request.line_end = 9_999;
    let err = match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Ok(_) => panic!("line number past length must error"),
        Err(err) => err,
    };
    assert!(err.message.contains("exceeds file line count") || err.message.contains("line_end"));
}

#[test]
fn a_line_number_within_the_files_real_length_succeeds() {
    let project = activated_project("lib.rs", b"line one\nline two\nline three\n");
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 2;
    request.line_end = 3;
    assert!(
        finding(
            &Parameters(request),
            &crate::progress::NoopProgressSink,
            &crate::sync::SourceSyncManager::default(),
        )
        .is_ok()
    );
}

#[test]
fn an_oversized_description_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = base_request(&project, "lib.rs");
    request.description = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
    let err = match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Ok(_) => panic!("oversized description must error"),
        Err(err) => err,
    };
    assert!(err.message.contains("description") || err.message.contains("character"));
}
#[test]
fn an_empty_file_cannot_have_a_finding() {
    // The fixture name deliberately avoids the word the old assertion looked
    // for, and the assertion is the exact message, so neither the file name
    // nor a fixture path can satisfy it on its own.
    let project = activated_project("blank.rs", b"");
    let request = base_request(&project, "blank.rs");
    let err = match finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) {
        Ok(_) => panic!("empty file finding must error"),
        Err(err) => err,
    };
    assert_eq!(err.message, "blank.rs has no lines to attach a finding to");
}

/// The session-scoped dedupe's core contract. Two calls that agree on the
/// conclusion's identity — file, content hash, line range, category, title —
/// must leave **one** row: the second call refreshes the existing row with the
/// latest body (severity, description) and returns the same `id`, rather than
/// appending a near-duplicate the way the unconditionally-`INSERT` build did.
#[test]
fn restating_a_conclusion_refreshes_one_row_with_the_latest_body() {
    let _session_guard = crate::util::lock_or_recover(&SESSION_TEST_LOCK);
    let project = activated_project("lib.rs", b"line one\nline two\n");

    let manager = crate::sync::SourceSyncManager::default();
    let first = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("first finding succeeds");

    let mut restated = base_request(&project, "lib.rs");
    restated.severity = "critical".to_owned();
    restated.description = "A sharper reading of the same conclusion.".to_owned();
    let second = finding(
        &Parameters(restated),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("restated finding succeeds");

    assert_eq!(
        second.0.id, first.0.id,
        "a restatement must refresh the existing row, not create a new one"
    );

    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let connection = crate::store::open_read_only(&db_path).expect("open db read-only");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count findings");
    assert_eq!(count, 1, "a restatement must not leave two rows");
    let (severity, description, title): (String, String, String) = connection
        .query_row(
            "SELECT severity, description, title FROM findings WHERE id = ?1",
            [first.0.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read refreshed row");
    assert_eq!(severity, "critical", "the restatement's severity must win");
    assert_eq!(description, "A sharper reading of the same conclusion.");
    assert_eq!(
        title, "Reviewed conclusion",
        "identity fields are unchanged"
    );
}

/// A different target is a different conclusion. Two calls on the same file
/// that name different line ranges must leave two rows, each keeping its own
/// id — dedupe must not collapse distinct subjects onto one row.
#[test]
fn distinct_targets_leave_distinct_rows() {
    let _session_guard = crate::util::lock_or_recover(&SESSION_TEST_LOCK);
    let project = activated_project("lib.rs", b"line one\nline two\n");
    let manager = crate::sync::SourceSyncManager::default();
    let first = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("first finding succeeds");

    let mut elsewhere = base_request(&project, "lib.rs");
    elsewhere.line_start = 2;
    elsewhere.line_end = 2;
    let second = finding(
        &Parameters(elsewhere),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("second finding succeeds");

    assert_ne!(
        second.0.id, first.0.id,
        "a different line range is a different finding"
    );
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let connection = crate::store::open_read_only(&db_path).expect("open db read-only");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count findings");
    assert_eq!(count, 2, "distinct targets must not be merged");
}

/// The content hash is part of the identity: a conclusion restated after its
/// file changed rests on different evidence, so the old row goes stale and the
/// restatement is a new row. Dedupe must not silently revive a stale row.
#[test]
fn a_restatement_after_the_file_changed_is_a_new_row() {
    let _session_guard = crate::util::lock_or_recover(&SESSION_TEST_LOCK);
    let project = activated_project("lib.rs", b"pub fn a() {}\n");

    let manager = crate::sync::SourceSyncManager::default();
    let first = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("first finding succeeds");

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let second = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("restated finding succeeds");

    assert_ne!(
        second.0.id, first.0.id,
        "changed evidence is a new identity, not a refresh"
    );
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let connection = crate::store::open_read_only(&db_path).expect("open db read-only");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count findings");
    assert_eq!(count, 2, "the stale row and the new row both remain");
    assert_eq!(finding_status(&connection, first.0.id), "stale");
    assert_eq!(finding_status(&connection, second.0.id), "current");
}

#[test]
fn a_finding_against_a_binary_file_carries_the_file_not_indexed_code() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    fs::write(project.path().join("blob.dat"), b"\0\x01binary\n").expect("binary file");
    let request = base_request(&project, "blob.dat");

    let result = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    let Err(error) = result else {
        panic!("a binary file has no indexed content");
    };
    let data = error.data.expect("a binary file carries a payload");
    assert_eq!(data["code"], "file_not_indexed", "{}", error.message);
}

fn validation_error(request: FindingRequest) -> ErrorData {
    let Err(error) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        panic!("the request must be refused");
    };
    error
}

#[test]
fn input_caps_refuse_an_oversized_path_file_and_zero_line() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");

    let mut path = base_request(&project, "lib.rs");
    path.path = "x".repeat(MAX_PATH_BYTES + 1);
    assert_eq!(validation_error(path).message, "path exceeds 32768 bytes");

    let mut file = base_request(&project, "lib.rs");
    file.file = "x".repeat(MAX_FILE_BYTES + 1);
    assert_eq!(validation_error(file).message, "file exceeds 8192 bytes");

    let mut line = base_request(&project, "lib.rs");
    line.line_start = 0;
    assert_eq!(
        validation_error(line).message,
        "line_start and line_end are one-based; zero is not a valid line"
    );
}

const FILES_SCHEMA: &str = "CREATE TABLE files (path TEXT, content_hash TEXT, content TEXT); \
     INSERT INTO files VALUES ('lib.rs', 'hash', 'line one');";

/// Runs `insert_finding` against a hand-written schema so one statement can
/// fail on its own, without a production test hook: the `files` row stays
/// readable while one `findings` column the statement names is absent
/// (schema drift).
fn insert_finding_error(request: &FindingRequest, schema: &str, seed: Option<&str>) -> ErrorData {
    let mut connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(&format!("{FILES_SCHEMA} {schema}"))
        .expect("create test schema");
    if let Some(seed) = seed {
        connection
            .execute(seed, [session_id().to_string()])
            .expect("seed the stored finding");
    }
    let tx = connection.transaction().expect("transaction");
    insert_finding(&tx, request, "rev").expect_err("the statement must fail")
}

/// Each fallible statement in `insert_finding` — the restatement lookup, the
/// restatement UPDATE, the `created_at_unix` read, and the INSERT — must
/// surface a database failure as an internal error, never as a bad input.
#[test]
fn each_findings_write_statement_failure_is_an_internal_error() {
    let _session_guard = crate::util::lock_or_recover(&SESSION_TEST_LOCK);
    let project = tempfile::tempdir().expect("temp dir");
    let request = base_request(&project, "lib.rs");
    let cases: &[(&str, &str, Option<&str>, &str)] = &[
        (
            "restatement lookup",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, title TEXT, description TEXT, \
             created_at_unix INTEGER, evidence_revision TEXT, status TEXT, session_id TEXT);",
            None,
            "no such column: category",
        ),
        (
            "restatement update",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, category TEXT, title TEXT, description TEXT, \
             created_at_unix INTEGER, evidence_revision TEXT, status TEXT, session_id TEXT);",
            Some(
                "INSERT INTO findings (path, source_hash, line_start, line_end, category, title, \
                 description, created_at_unix, evidence_revision, status, session_id) \
                 VALUES ('lib.rs', 'hash', 1, 1, 'correctness', 'Reviewed conclusion', 'old', 1, \
                 'rev', 'current', ?1)",
            ),
            "no such column: severity",
        ),
        (
            "created_at read",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, severity TEXT, category TEXT, title TEXT, \
             description TEXT, evidence_revision TEXT, status TEXT, session_id TEXT);",
            None,
            "no such column: created_at_unix",
        ),
        (
            "finding insert",
            "CREATE TABLE findings (id INTEGER PRIMARY KEY, path TEXT, source_hash TEXT, \
             line_start INTEGER, line_end INTEGER, severity TEXT, category TEXT, title TEXT, \
             description TEXT, created_at_unix INTEGER, status TEXT, session_id TEXT);",
            None,
            "evidence_revision",
        ),
    ];

    for &(label, schema, seed, expected) in cases {
        let error = insert_finding_error(&request, schema, seed);
        assert_eq!(
            error.code,
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "{label}"
        );
        assert!(
            error.message.contains(expected),
            "{label}: {}",
            error.message
        );
    }
}

/// A database that cannot be read is not the caller's `file` argument being
/// wrong: it must be an internal error that does not blame the input.
#[test]
fn a_failed_files_read_is_an_internal_error_not_a_bad_input() {
    let mut connection = Connection::open_in_memory().expect("in-memory database");
    let tx = connection.transaction().expect("transaction");

    let error = fetch_file(&tx, "lib.rs").expect_err("a missing files table must fail");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.contains("no such table: files"),
        "{}",
        error.message
    );
    assert!(!error.message.contains("lib.rs"), "{}", error.message);
}
