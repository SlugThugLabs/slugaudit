//! The adversarial half of the `query` tool's boundary. Every test here
//! asserts *rejection* — the plan's wording, "asserting rejection, not just
//! 'no rows'". A silently empty page and a statement refused before
//! execution are indistinguishable to a caller that never inspects the
//! error, so nothing here is satisfied by an `Ok` with no rows, and every
//! test that claims nothing changed re-reads the index through the same
//! tool rather than trusting the refusal. Split out of `query_tests.rs`
//! (one property per test, and the payload tables are the bulk of it) to
//! keep that file inside the test-file code-line cap.
use super::*;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;

/// The fixture every payload below attacks: one indexed `lib.rs`.
fn project() -> tempfile::TempDir {
    activated_project("lib.rs", b"pub fn a() {}\n")
}

fn ask(project: &tempfile::TempDir, sql: &str) -> Result<QueryResponse, ErrorData> {
    query(
        &Parameters(QueryRequest {
            path: project.path().to_string_lossy().into_owned(),
            sql: sql.to_owned(),
            offset: 0,
            expected_revision: None,
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

/// Demands that `sql` be refused. The panic names the false pass: an `Ok`
/// here would mean a tool advertising a read-only connection executed the
/// payload anyway.
fn refused(project: &tempfile::TempDir, sql: &str, what: &str) -> ErrorData {
    match ask(project, sql) {
        Err(error) => error,
        Ok(response) => panic!(
            "`{sql}` was accepted and returned {} row(s) instead of an error; a false pass here \
             means {what} was not actually refused",
            response.rows.len()
        ),
    }
}

/// Refuses every payload, then re-reads the fixture through the same tool:
/// a refusal response alone cannot prove the index is unchanged.
fn all_refused(project: &tempfile::TempDir, payloads: &[&str], what: &str) {
    for sql in payloads {
        refused(project, sql, what);
    }
    let after = ask(project, "SELECT path, language FROM files").expect("the index still reads");
    assert_eq!(
        after.rows,
        vec![serde_json::json!({"path": "lib.rs", "language": "rust"})],
        "{what}: the index changed, so a refused payload still had an effect"
    );
}

/// Two shapes of CTE write, both refused and both pinned by re-reading the
/// index: a data-modifying CTE referenced by a `SELECT`, and a CTE feeding
/// a top-level `INSERT`/`UPDATE`/`DELETE ... RETURNING`. The second shape
/// parses on its own, so it is refused by the tool's wrapper — and, with
/// the wrapper experimentally removed, by the read-only connection itself
/// ("attempt to write a readonly database"), which is the guard the
/// contract claims writes rest on.
#[test]
fn writes_through_a_data_modifying_cte_are_refused() {
    let project = project();

    all_refused(
        &project,
        &[
            "WITH x AS (INSERT INTO files(path, language) VALUES ('planted.rs', 'rust') RETURNING path) SELECT * FROM x",
            "WITH x AS (UPDATE files SET path = 'planted.rs' RETURNING path) SELECT * FROM x",
            "WITH x AS (DELETE FROM files RETURNING path) SELECT * FROM x",
            "WITH RECURSIVE x AS (DELETE FROM files RETURNING path) SELECT * FROM x",
            "WITH x AS (SELECT 1 AS a) INSERT INTO files(path, language) SELECT 'planted.rs', 'rust' FROM x RETURNING path",
            "WITH x AS (SELECT 1 AS a) UPDATE files SET path = 'planted.rs' RETURNING path",
            "WITH x AS (SELECT 1 AS a) DELETE FROM files RETURNING path",
        ],
        "a write through a data-modifying CTE",
    );
}

/// The same write attempt one nesting level deeper: a scalar subquery and
/// an `EXISTS` subquery are both read-shaped positions, and a write must
/// not become reachable by wrapping it in one.
#[test]
fn writes_through_a_scalar_or_exists_subquery_are_refused() {
    let project = project();

    all_refused(
        &project,
        &[
            "SELECT (WITH x AS (DELETE FROM files RETURNING path) SELECT path FROM x) AS smuggled",
            "SELECT EXISTS (SELECT 1 FROM (WITH x AS (UPDATE files SET path = 'planted.rs' RETURNING path) SELECT path FROM x)) AS smuggled",
            "SELECT (SELECT (WITH x AS (INSERT INTO files(path, language) VALUES ('p.rs', 'rust') RETURNING path) SELECT path FROM x)) AS smuggled",
        ],
        "a write through a scalar or EXISTS subquery",
    );
}

/// `PRAGMA` is a statement kind, not a `SELECT`, so the wrapper refuses
/// both its read and write forms — as it refuses every other statement
/// kind the tool refuses to express. The knob values are re-read through
/// the tool afterwards, so a write form that somehow took effect before
/// the error is a failure, not a shrug.
#[test]
fn pragma_and_other_non_select_statements_smuggled_into_the_subquery_are_refused() {
    let project = project();
    let knobs =
        "SELECT (SELECT * FROM pragma_user_version) AS v, (SELECT * FROM pragma_journal_mode) AS m";
    let before = ask(&project, knobs).expect("read the writable knobs");

    all_refused(
        &project,
        &[
            "PRAGMA table_info(files)",
            "PRAGMA schema_version",
            "PRAGMA main.table_info(files)",
            "PRAGMA user_version = 7",
            "PRAGMA journal_mode = DELETE",
            "PRAGMA writable_schema = ON",
            "PRAGMA query_only = OFF",
            "VACUUM",
            "VACUUM INTO '/tmp/slugaudit-vacuum.db'",
            "ANALYZE",
            "REINDEX",
            "CREATE TABLE planted (x)",
            "DROP TABLE files",
            "BEGIN",
            "COMMIT",
            "SAVEPOINT s",
            "EXPLAIN SELECT 1",
        ],
        "a non-SELECT statement smuggled into the subquery",
    );

    // `VACUUM INTO` is the one payload here that would leave a whole
    // database file behind at an arbitrary path, so the refusal is checked
    // against the filesystem too.
    assert!(
        !std::path::Path::new("/tmp/slugaudit-vacuum.db").exists(),
        "a refused VACUUM INTO still wrote a database file to /tmp"
    );

    let after = ask(&project, knobs).expect("re-read the writable knobs");
    assert_eq!(
        after.rows, before.rows,
        "a refused write-form PRAGMA still changed a knob"
    );
}

/// `load_extension` is an ordinary `SELECT`, so the wrapper cannot refuse
/// it: what stops it is SQLite's own disabled-by-default extension
/// loading. The filesystem functions the CLI builds ship are not compiled
/// into this library at all. `writefile` is the one payload here that
/// could leave a trace, so the absence of that file is asserted directly.
#[test]
fn functions_that_could_leave_the_sandbox_are_refused_or_inert() {
    let project = project();
    let escaped = std::path::Path::new("/tmp/slugaudit-escape.txt");

    for sql in [
        "SELECT load_extension('/tmp/slugaudit-escape.so') AS x",
        "SELECT load_extension('/tmp/slugaudit-escape.so', 'sqlite3_extension_init') AS x",
        "SELECT readfile('/etc/passwd') AS x",
        "SELECT writefile('/tmp/slugaudit-escape.txt', 'escaped') AS x",
        "SELECT edit('/etc/passwd') AS x",
    ] {
        refused(
            &project,
            sql,
            "a function that reads or writes the filesystem",
        );
    }
    assert!(
        !escaped.exists(),
        "a refused writefile() payload still created {escaped:?}"
    );

    // `fts3_tokenizer` is compiled in but disabled, which is a boundary in
    // both directions: the read form is an inert NULL rather than a raw
    // pointer, and the form that installs a pointer errors. If a build ever
    // re-enables it, the first assertion fails on a leaked pointer.
    let reader = ask(&project, "SELECT fts3_tokenizer('simple') AS t").expect("reader form runs");
    assert_eq!(
        reader.rows[0]["t"],
        serde_json::Value::Null,
        "fts3_tokenizer returned a value instead of NULL: {:?}",
        reader.rows[0]
    );
    let installer = ask(
        &project,
        "SELECT fts3_tokenizer('simple', zeroblob(8)) AS t",
    );
    assert!(
        installer.is_err(),
        "the pointer-installing fts3_tokenizer form must be refused, got: {:?}",
        installer.ok()
    );
}

/// Statement splicing. A real top-level `;` is caught by the separator
/// scanner with the friendly typed message — even when a string literal or
/// a comment sits immediately before it. A `;` *inside* a comment hides
/// the splice from that scanner, so the whole text reaches the wrapper as
/// one statement: the wrapper is then the guard that refuses it.
#[test]
fn a_second_statement_cannot_be_spliced_past_the_single_statement_guard() {
    let project = project();

    for sql in [
        "SELECT 1; DELETE FROM files",
        "SELECT 'literal;text' AS x; DELETE FROM files",
        "SELECT 1 -- comment\n; DELETE FROM files",
        "SELECT 1 /* comment */; DELETE FROM files",
    ] {
        let error = refused(&project, sql, "a second statement spliced after the first");
        assert!(
            error.message.contains("more than one statement"),
            "{sql}: expected the separator-scanner refusal, got: {}",
            error.message
        );
    }

    all_refused(
        &project,
        &[
            "SELECT 1 -- ;\nDELETE FROM files",
            "SELECT 1 /* ; */ DELETE FROM files",
            "DELETE FROM files -- trailing comment",
            "DELETE FROM files;",
        ],
        "a write spliced past the separator scanner",
    );
}

/// The other half of the boundary, and the reason this suite's rejections
/// are not vacuous: `sqlite_master` and the `pragma_*` table-valued
/// functions are exactly what the server instructions tell the agent to
/// run, and an empty result is a *successful* read, not a refusal. If this
/// test fails, the suite above is passing because everything fails.
#[test]
fn legal_reads_of_the_schema_and_pragma_table_functions_are_allowed() {
    let project = project();

    let tables = ask(
        &project,
        "SELECT name FROM sqlite_master WHERE type = 'table'",
    )
    .expect("sqlite_master reads");
    assert!(
        tables.rows.iter().any(|row| row["name"] == "files"),
        "legal schema introspection must work; a false pass here means ordinary reads are \
         being refused too: {:?}",
        tables.rows
    );

    let columns = ask(&project, "SELECT name FROM pragma_table_info('files')")
        .expect("pragma table-valued functions read");
    assert!(
        columns.rows.iter().any(|row| row["name"] == "path"),
        "pragma table-valued functions must work; a false pass here means every SELECT is \
         being refused: {:?}",
        columns.rows
    );

    let empty = ask(&project, "SELECT path FROM files WHERE 0").expect("an empty read succeeds");
    assert!(
        empty.rows.is_empty(),
        "an empty read returns Ok, not an error; a false pass here means this suite cannot \
         tell 'no rows' from 'refused': {:?}",
        empty.rows
    );
}

/// The request schema says only SELECT is accepted. `PRAGMA table_info` and
/// `EXPLAIN SELECT 1` are read-only, but neither is a SELECT, so the wrapper
/// refuses each before execution with the compile error SQLite reports for
/// the wrapped statement.
#[test]
fn read_only_non_select_statements_are_refused_by_the_wrapper() {
    let project = project();

    for (sql, expected) in [
        (
            "PRAGMA table_info(files)",
            "near \"(\": syntax error in SELECT * FROM (\nPRAGMA table_info(files)\n) \
             LIMIT -1 OFFSET 0 at offset 33",
        ),
        (
            "EXPLAIN SELECT 1",
            "near \"SELECT\": syntax error in SELECT * FROM (\nEXPLAIN SELECT 1\n) \
             LIMIT -1 OFFSET 0 at offset 24",
        ),
    ] {
        let error = refused(&project, sql, "a read-only non-SELECT statement");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{sql}");
        assert_eq!(error.message.as_ref(), expected, "{sql}");
        assert!(
            error.data.is_none(),
            "{sql}: a wrapper refusal carries no machine-readable code"
        );
    }
}

#[test]
fn a_comments_only_statement_is_refused_rather_than_run_as_an_empty_query() {
    let project = project();

    let error = refused(
        &project,
        "-- only a comment, no statement",
        "a comments-only statement",
    );

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "sql must contain a statement, not only comments"
    );
}

#[test]
fn a_query_path_over_the_cap_is_refused_before_any_sync() {
    let error = query_with_limits(
        &Parameters(QueryRequest {
            path: "x".repeat(32 * 1024 + 1),
            sql: "SELECT 1".to_owned(),
            offset: 0,
            expected_revision: None,
            page_bytes: None,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("an oversized path must be refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message.as_ref(), "path exceeds 32768 bytes");
}

#[test]
fn an_expected_revision_over_the_query_cap_is_refused_before_any_sync() {
    let directory = tempfile::tempdir().expect("project dir");
    let error = query_with_limits(
        &Parameters(QueryRequest {
            path: directory.path().to_string_lossy().into_owned(),
            sql: "SELECT 1".to_owned(),
            offset: 0,
            expected_revision: Some("x".repeat(257)),
            page_bytes: None,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("an oversized expected_revision must be refused");

    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.message.as_ref(),
        "expected_revision exceeds 256 bytes"
    );
}
