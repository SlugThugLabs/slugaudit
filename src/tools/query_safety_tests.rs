//! The adversarial half of the `query` tool's boundary. Every test here
//! asserts *rejection*, not just "no rows". A silently empty page and a
//! statement refused before execution are indistinguishable to a caller that
//! never inspects the error, so nothing here is satisfied by an `Ok` with no rows, and every
//! test that claims nothing changed re-reads the index through the same
//! tool rather than trusting the refusal. Kept apart from `query_tests.rs`
//! (one property per test, and the payload tables are the bulk of it) to
//! keep that file inside the test-file code-line cap.
use super::*;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::time::Duration;

/// The fixture every payload below attacks: one indexed `lib.rs`.
pub(super) fn project() -> TestResult<tempfile::TempDir> {
    activated_project("lib.rs", b"pub fn a() {}\n")
}

pub(super) fn ask(project: &tempfile::TempDir, sql: &str) -> Result<QueryResponse, ErrorData> {
    query(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

/// Demands that `sql` be refused. The error names the false pass: an `Ok`
/// here would mean a tool advertising read-only SQL executed the payload
/// anyway.
pub(super) fn refused(project: &tempfile::TempDir, sql: &str, what: &str) -> TestResult<ErrorData> {
    match ask(project, sql) {
        Err(error) => Ok(error),
        Ok(response) => Err(format!(
            "`{sql}` was accepted and returned {} row(s) instead of an error; a false pass here \
             means {what} was not actually refused",
            response.rows.len()
        ))
        .check("the SQL is refused"),
    }
}

/// Refuses every payload, then re-reads the fixture through the same tool:
/// a refusal response alone cannot prove the index is unchanged.
fn all_refused(project: &tempfile::TempDir, payloads: &[&str], what: &str) -> TestResult {
    for sql in payloads {
        refused(project, sql, what)?;
    }
    let after = ask(project, "SELECT path, language FROM files").check("the index still reads")?;
    ensure_eq(
        &after.rows,
        &vec![serde_json::json!({"path": "lib.rs", "language": "rust"})],
        format!("{what}: the index changed, so a refused payload still had an effect"),
    )?;
    Ok(())
}

/// Two shapes of CTE write, both refused and both pinned by re-reading the
/// index: a data-modifying CTE referenced by a `SELECT`, and a CTE feeding
/// a top-level `INSERT`/`UPDATE`/`DELETE ... RETURNING`. The second shape
/// parses on its own, so it is refused by the tool's wrapper — and, with
/// the wrapper experimentally removed, by the read-only connection itself
/// ("attempt to write a readonly database"), a second layer that is a
/// connection mode rather than a trust boundary.
#[test]
fn writes_through_a_data_modifying_cte_are_refused() -> TestResult {
    let project = project()?;

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
    )?;
    Ok(())
}

/// The same write attempt one nesting level deeper: a scalar subquery and
/// an `EXISTS` subquery are both read-shaped positions, and a write must
/// not become reachable by wrapping it in one.
#[test]
fn writes_through_a_scalar_or_exists_subquery_are_refused() -> TestResult {
    let project = project()?;

    all_refused(
        &project,
        &[
            "SELECT (WITH x AS (DELETE FROM files RETURNING path) SELECT path FROM x) AS smuggled",
            "SELECT EXISTS (SELECT 1 FROM (WITH x AS (UPDATE files SET path = 'planted.rs' RETURNING path) SELECT path FROM x)) AS smuggled",
            "SELECT (SELECT (WITH x AS (INSERT INTO files(path, language) VALUES ('p.rs', 'rust') RETURNING path) SELECT path FROM x)) AS smuggled",
        ],
        "a write through a scalar or EXISTS subquery",
    )?;
    Ok(())
}

/// `PRAGMA` is a statement kind, not a `SELECT`, so the wrapper refuses
/// both its read and write forms — as it refuses every other statement
/// kind the tool refuses to express. The knob values are re-read through
/// the tool afterwards, so a write form that somehow took effect before
/// the error is a failure, not a shrug.
#[test]
fn pragma_and_other_non_select_statements_smuggled_into_the_subquery_are_refused() -> TestResult {
    let project = project()?;
    let before = writable_knobs(&project)?;

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
    )?;

    // `VACUUM INTO` is the one payload here that would leave a whole
    // database file behind at an arbitrary path, so the refusal is checked
    // against the filesystem too.
    ensure(
        !std::path::Path::new("/tmp/slugaudit-vacuum.db").exists(),
        "a refused VACUUM INTO still wrote a database file to /tmp",
    )?;

    ensure_eq(
        &writable_knobs(&project)?,
        &before,
        "a refused write-form PRAGMA still changed a knob",
    )?;
    Ok(())
}

/// `user_version` and `journal_mode`, read on a plain connection. A first
/// `query` syncs the project so the file exists.
fn writable_knobs(project: &tempfile::TempDir) -> TestResult<(i64, String)> {
    ask(project, "SELECT 1 AS synced").check("sync the project")?;
    let database = project.path().join(".planning/slugaudit/project.db");
    let connection =
        rusqlite::Connection::open(database).check("open the project database directly")?;
    connection
        .query_row(
            "SELECT (SELECT * FROM pragma_user_version), (SELECT * FROM pragma_journal_mode)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .check("read the knobs")
}

/// `load_extension` is an ordinary `SELECT`, so the wrapper cannot refuse
/// it: what stops it is SQLite's own disabled-by-default extension
/// loading. The filesystem functions the CLI builds ship are not compiled
/// into this library at all. `writefile` is the one payload here that
/// could leave a trace, so the absence of that file is asserted directly.
#[test]
fn functions_that_could_leave_the_sandbox_are_refused_or_inert() -> TestResult {
    let project = project()?;
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
        )?;
    }
    ensure(
        !escaped.exists(),
        format!("a refused writefile() payload still created {escaped:?}"),
    )?;

    // `fts3_tokenizer` is compiled in but disabled, which is a boundary in
    // both directions: the read form is an inert NULL rather than a raw
    // pointer, and the form that installs a pointer errors. If a build ever
    // re-enables it, the first assertion fails on a leaked pointer.
    let reader = ask(&project, "SELECT fts3_tokenizer('simple') AS t").check("reader form runs")?;
    ensure_eq(
        reader
            .rows
            .first()
            .check("reader.rows[0]")?
            .get("t")
            .check("reader.rows[0][\"t\"]")?,
        &serde_json::Value::Null,
        format!(
            "fts3_tokenizer returned a value instead of NULL: {:?}",
            reader.rows.first().check("reader.rows[0]")?
        ),
    )?;
    let installer = ask(
        &project,
        "SELECT fts3_tokenizer('simple', zeroblob(8)) AS t",
    );
    ensure(
        installer.is_err(),
        format!("the pointer-installing fts3_tokenizer form must be refused, got: {installer:?}"),
    )?;
    Ok(())
}

/// Statement splicing. A real top-level `;` followed by a second statement
/// is caught by SQLite's own parser (`MultipleStatement`) with the friendly
/// typed message — even when a string literal or a comment sits immediately
/// before it. A `;` *inside* a comment is not a separator, so the whole text
/// reaches the wrapper as one statement: the wrapper then refuses it.
#[test]
fn a_second_statement_cannot_be_spliced_past_the_single_statement_guard() -> TestResult {
    let project = project()?;

    for sql in [
        "SELECT 1; DELETE FROM files",
        "SELECT 'literal;text' AS x; DELETE FROM files",
        "SELECT 1 -- comment\n; DELETE FROM files",
        "SELECT 1 /* comment */; DELETE FROM files",
    ] {
        let error = refused(&project, sql, "a second statement spliced after the first")?;
        ensure(
            error.message.contains("more than one statement"),
            format!(
                "{sql}: expected the single-statement refusal, got: {}",
                error.message
            ),
        )?;
    }

    all_refused(
        &project,
        &[
            "SELECT 1 -- ;\nDELETE FROM files",
            "SELECT 1 /* ; */ DELETE FROM files",
            "DELETE FROM files -- trailing comment",
            "DELETE FROM files;",
        ],
        "a write spliced past the single-statement check",
    )?;
    Ok(())
}

/// The other half of the boundary, and the reason this suite's rejections
/// are not vacuous: `sqlite_master` and the `pragma_*` table-valued
/// functions are exactly what the server instructions tell the agent to
/// run, and an empty result is a *successful* read, not a refusal. If this
/// test fails, the suite above is passing because everything fails.
#[test]
fn legal_reads_of_the_schema_and_pragma_table_functions_are_allowed() -> TestResult {
    let project = project()?;

    let tables = ask(
        &project,
        "SELECT name FROM sqlite_master WHERE type = 'table'",
    )
    .check("sqlite_master reads")?;
    ensure(
        tables.rows.iter().any(|row| row["name"] == "files"),
        format!(
            "legal schema introspection must work; a false pass here means ordinary reads are \
         being refused too: {:?}",
            tables.rows
        ),
    )?;

    let columns = ask(&project, "SELECT name FROM pragma_table_info('files')")
        .check("pragma table-valued functions read")?;
    ensure(
        columns.rows.iter().any(|row| row["name"] == "path"),
        format!(
            "pragma table-valued functions must work; a false pass here means every SELECT is \
         being refused: {:?}",
            columns.rows
        ),
    )?;

    let empty = ask(&project, "SELECT path FROM files WHERE 0").check("an empty read succeeds")?;
    ensure(
        empty.rows.is_empty(),
        format!(
            "an empty read returns Ok, not an error; a false pass here means this suite cannot \
         tell 'no rows' from 'refused': {:?}",
            empty.rows
        ),
    )?;
    Ok(())
}

/// The request schema says only SELECT is accepted. A PRAGMA is refused
/// before compilation (it can change process state while being prepared);
/// `EXPLAIN SELECT 1` is refused by the wrapper, with SQLite's compile
/// error for the wrapped statement kept.
#[test]
fn read_only_non_select_statements_are_refused_by_the_wrapper() -> TestResult {
    let project = project()?;
    let refusal = "sql was refused: `query` accepts a single read-only SELECT/WITH statement, \
                   and this is another kind of statement (PRAGMA, EXPLAIN, a write, DDL or a \
                   transaction command), so it cannot run as a subquery (SQLite: ";

    let pragma = refused(&project, "PRAGMA table_info(files)", "a PRAGMA statement")?;
    ensure_eq(
        &pragma.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "pragma.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &pragma.message.as_ref(),
        &"sql was refused: `query` accepts a single read-only SELECT/WITH statement, not \
         PRAGMA (a PRAGMA can change process state while being prepared)",
        "pragma.message.as_ref() == \"sql was refused: `query` accepts a single read-only SELECT/WITH statement, not \\ PRAGMA (a PRAGMA can change process state while being prepared)\"",
    )?;
    ensure(
        pragma.data.is_none(),
        "a refusal carries no machine-readable code",
    )?;

    let explain = refused(&project, "EXPLAIN SELECT 1", "a non-SELECT statement")?;
    ensure_eq(
        &explain.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "explain.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &explain.message.as_ref(),
        &format!(
            "{refusal}near \"SELECT\": syntax error in SELECT * FROM (\nEXPLAIN SELECT \
             1\n) LIMIT -1 OFFSET 0 at offset 24)"
        ),
        "explain.message.as_ref() == format!( \"{refusal}near \\\"SELECT\\\": syntax error in SELECT * FROM (\\nEXPLAIN SELECT \\ 1\\n) LIMIT -1 OFFSET 0 at offset 24)\" )",
    )?;
    ensure(
        explain.data.is_none(),
        "a refusal carries no machine-readable code",
    )?;
    Ok(())
}

#[test]
fn a_comments_only_statement_is_refused_rather_than_run_as_an_empty_query() -> TestResult {
    let project = project()?;

    let error = refused(
        &project,
        "-- only a comment, no statement",
        "a comments-only statement",
    )?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"sql must contain a statement, not only comments",
        "error.message.as_ref() == \"sql must contain a statement, not only comments\"",
    )?;
    Ok(())
}

#[test]
fn a_query_path_over_the_cap_is_refused_before_any_sync() -> TestResult {
    let error = query_with_limits(
        &Parameters(QueryRequest {
            path: Some("x".repeat(32 * 1024 + 1)),
            sql: "SELECT 1".to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("an oversized path must be refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"path exceeds 32768 bytes",
        "error.message.as_ref() == \"path exceeds 32768 bytes\"",
    )?;
    Ok(())
}

/// Demands the clear refusal: `invalid_params`, saying what `query`
/// accepts, never the comments-only message.
fn assert_refused_as_not_a_select(error: &ErrorData, sql: &str) -> TestResult {
    let opening = "sql was refused: `query` accepts a single read-only SELECT/WITH statement";
    ensure_eq(&error.code, &rmcp::model::ErrorCode::INVALID_PARAMS, sql)?;
    ensure(
        error.message.starts_with(opening) && !error.message.contains("only comments"),
        format!("{sql}: unexpected message: {}", error.message),
    )?;
    Ok(())
}

/// A process-wide pragma as a second, freshly opened connection reads it
/// (`None` when it has no value).
pub(super) fn fresh_pragma(pragma: &str) -> TestResult<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(rusqlite::Connection::open_in_memory()
        .check("a freshly opened connection")?
        .query_row(&format!("PRAGMA {pragma}"), [], |row| {
            row.get::<_, rusqlite::types::Value>(0)
        })
        .optional()
        .check("read the pragma")?
        .map(|value| format!("{value:?}")))
}

/// SQLite applies these pragmas while *preparing* the statement and each is
/// process-wide: a refusal that came after the prepare would leave, for
/// `hard_heap_limit=1`, every later database open in the process failing.
/// A fresh connection reads each value unchanged and a `report` still runs.
#[test]
fn refused_prepare_time_pragmas_leave_process_wide_state_unchanged() -> TestResult {
    let project = project()?;
    let elsewhere = tempfile::tempdir().check("a directory the pragma would point at")?;

    for (pragma, setter) in [
        ("hard_heap_limit", "PRAGMA hard_heap_limit=1".to_owned()),
        (
            "soft_heap_limit",
            "PRAGMA soft_heap_limit=123456789".to_owned(),
        ),
        (
            "temp_store_directory",
            format!(
                "PRAGMA temp_store_directory='{}'",
                elsewhere.path().display()
            ),
        ),
    ] {
        let before = fresh_pragma(pragma)?;
        let error = refused(&project, &setter, "a prepare-time PRAGMA")?;
        assert_refused_as_not_a_select(&error, &setter)?;
        ensure_eq(
            &fresh_pragma(pragma)?,
            &before,
            format!("{setter} took effect"),
        )?;
    }

    ensure_eq(
        &fresh_pragma("hard_heap_limit")?.as_deref(),
        &Some("Integer(0)"),
        "fresh_pragma(\"hard_heap_limit\")?.as_deref() == Some(\"Integer(0)\")",
    )?;
    let report = crate::tools::report::report(
        &Parameters(crate::tools::report::ReportRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );
    report.as_ref().check("`report` is ok")?;
    Ok(())
}

/// The `pragma_*` table functions are ordinary SELECTs: SQLite exposes as
/// table-valued functions only the pragmas that return results and have no
/// side effects, so reading one is allowed by design.
#[test]
fn pragma_table_valued_functions_are_ordinary_selects() -> TestResult {
    let project = project()?;

    let columns =
        ask(&project, "SELECT * FROM pragma_table_info('files')").check("introspection reads")?;
    let version = ask(&project, "SELECT * FROM pragma_user_version")
        .check("a side-effect-free pragma function reads")?;

    ensure(
        columns
            .rows
            .iter()
            .any(|row| row["name"] == "path" && row["cid"].is_i64()),
        format!("{:?}", columns.rows),
    )?;
    ensure_eq(&version.rows.len(), &1, format!("{:?}", version.rows))?;
    ensure(
        version
            .rows
            .first()
            .check("version.rows[0]")?
            .get("user_version")
            .check("version.rows[0][\"user_version\"]")?
            .is_i64(),
        format!("{:?}", version.rows),
    )?;
    Ok(())
}

#[path = "query_safety_limits_tests.rs"]
mod limits_tests;
