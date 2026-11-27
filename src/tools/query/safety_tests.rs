//! Read-only and single-statement input bounds for `query`: preventing
//! accidental write, DDL, `PRAGMA`, `EXPLAIN`, or multi-statement SQL while
//! keeping `sqlite_master` and `pragma_*` table-valued functions available.
//!
//! Every rejection test asserts an explicit error (never silently returning an
//! empty `Ok` page) and re-reads the index afterward to confirm nothing changed.

use super::*;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

/// Fixture with one indexed `lib.rs`.
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

/// Demands that `sql` be refused with `ErrorData`.
pub(super) fn refused(project: &tempfile::TempDir, sql: &str, what: &str) -> TestResult<ErrorData> {
    match ask(project, sql) {
        Err(error) => Ok(error),
        Ok(response) => Err(format!(
            "`{sql}` was accepted and returned {} row(s) instead of an error; \
             {what} was not refused",
            response.rows.len()
        ))
        .check("the SQL is refused"),
    }
}

fn all_refused(project: &tempfile::TempDir, statements: &[&str], what: &str) -> TestResult {
    for sql in statements {
        refused(project, sql, what)?;
    }
    let after = ask(project, "SELECT path, language FROM files").check("the index still reads")?;
    ensure_eq(
        &after.rows,
        &vec![serde_json::json!({"path": "lib.rs", "language": "rust"})],
        format!("{what}: the index changed after a refused statement"),
    )?;
    Ok(())
}

/// Data-modifying CTEs — whether referenced by a `SELECT` or feeding a
/// top-level `INSERT`/`UPDATE`/`DELETE ... RETURNING` — are refused without
/// mutating the index.
#[test]
fn writes_through_a_data_modifying_cte_are_refused() -> TestResult {
    let project = project()?;

    all_refused(
        &project,
        &[
            "WITH x AS (INSERT INTO files(path, language) VALUES ('extra.rs', 'rust') RETURNING path) SELECT * FROM x",
            "WITH x AS (UPDATE files SET path = 'extra.rs' RETURNING path) SELECT * FROM x",
            "WITH x AS (DELETE FROM files RETURNING path) SELECT * FROM x",
            "WITH RECURSIVE x AS (DELETE FROM files RETURNING path) SELECT * FROM x",
            "WITH x AS (SELECT 1 AS a) INSERT INTO files(path, language) SELECT 'extra.rs', 'rust' FROM x RETURNING path",
            "WITH x AS (SELECT 1 AS a) UPDATE files SET path = 'extra.rs' RETURNING path",
            "WITH x AS (SELECT 1 AS a) DELETE FROM files RETURNING path",
        ],
        "a write through a data-modifying CTE",
    )?;
    Ok(())
}

#[test]
fn writes_through_a_scalar_or_exists_subquery_are_refused() -> TestResult {
    let project = project()?;

    all_refused(
        &project,
        &[
            "SELECT (WITH x AS (DELETE FROM files RETURNING path) SELECT path FROM x) AS nested",
            "SELECT EXISTS (SELECT 1 FROM (WITH x AS (UPDATE files SET path = 'extra.rs' RETURNING path) SELECT path FROM x)) AS nested",
            "SELECT (SELECT (WITH x AS (INSERT INTO files(path, language) VALUES ('p.rs', 'rust') RETURNING path) SELECT path FROM x)) AS nested",
        ],
        "a write through a scalar or EXISTS subquery",
    )?;
    Ok(())
}

#[test]
fn pragma_and_other_non_select_statements_are_refused() -> TestResult {
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
            "ANALYZE",
            "REINDEX",
            "CREATE TABLE scratch (x)",
            "DROP TABLE files",
            "BEGIN",
            "COMMIT",
            "SAVEPOINT s",
            "EXPLAIN SELECT 1",
        ],
        "a non-SELECT statement",
    )?;

    ensure_eq(
        &writable_knobs(&project)?,
        &before,
        "a refused write-form PRAGMA changed a database setting",
    )?;
    Ok(())
}

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

#[test]
fn a_second_statement_is_refused_by_the_single_statement_check() -> TestResult {
    let project = project()?;

    for sql in [
        "SELECT 1; DELETE FROM files",
        "SELECT 'literal;text' AS x; DELETE FROM files",
        "SELECT 1 -- comment\n; DELETE FROM files",
        "SELECT 1 /* comment */; DELETE FROM files",
    ] {
        let error = refused(&project, sql, "a second statement after the first")?;
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
        "a non-SELECT statement with comments or trailing semicolon",
    )?;
    Ok(())
}

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
        format!("schema introspection must work: {:?}", tables.rows),
    )?;

    let columns = ask(&project, "SELECT name FROM pragma_table_info('files')")
        .check("pragma table-valued functions read")?;
    ensure(
        columns.rows.iter().any(|row| row["name"] == "path"),
        format!(
            "pragma table-valued functions must work: {:?}",
            columns.rows
        ),
    )?;

    let empty = ask(&project, "SELECT path FROM files WHERE 0").check("an empty read succeeds")?;
    ensure(
        empty.rows.is_empty(),
        format!("an empty read returns Ok with zero rows: {:?}", empty.rows),
    )?;
    Ok(())
}

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
        "pragma.code == INVALID_PARAMS",
    )?;
    ensure_eq(
        &pragma.message.as_ref(),
        &"sql was refused: `query` accepts a single read-only SELECT/WITH statement, not \
         PRAGMA (a PRAGMA can change process state while being prepared)",
        "pragma refusal message",
    )?;
    ensure(
        pragma.data.is_none(),
        "a refusal carries no machine-readable code",
    )?;

    let explain = refused(&project, "EXPLAIN SELECT 1", "a non-SELECT statement")?;
    ensure_eq(
        &explain.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "explain.code == INVALID_PARAMS",
    )?;
    ensure_eq(
        &explain.message.as_ref(),
        &format!(
            "{refusal}near \"SELECT\": syntax error in SELECT * FROM (\nEXPLAIN SELECT \
             1\n) LIMIT -1 OFFSET 0 at offset 24)"
        ),
        "explain refusal message",
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
        "error.code == INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"sql must contain a statement, not only comments",
        "comments-only refusal message",
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
        "error.code == INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message.as_ref(),
        &"path exceeds 32768 bytes",
        "error.message.as_ref() == \"path exceeds 32768 bytes\"",
    )?;
    Ok(())
}

fn assert_refused_as_not_a_select(error: &ErrorData, sql: &str) -> TestResult {
    let opening = "sql was refused: `query` accepts a single read-only SELECT/WITH statement";
    ensure_eq(&error.code, &rmcp::model::ErrorCode::INVALID_PARAMS, sql)?;
    ensure(
        error.message.starts_with(opening) && !error.message.contains("only comments"),
        format!("{sql}: unexpected message: {}", error.message),
    )?;
    Ok(())
}

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
        "hard_heap_limit unchanged",
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

#[test]
fn a_prepare_time_pragma_after_another_statement_is_refused_unapplied() -> TestResult {
    let project = project()?;
    let before = fresh_pragma("hard_heap_limit")?;

    for sql in [
        "SELECT 1; PRAGMA hard_heap_limit=1024",
        "SELECT 1;\n-- note\n  pragma hard_heap_limit=1024",
        "SELECT 1; /* c */ PRAGMA hard_heap_limit=1024;",
        "SELECT ';'; PRAGMA hard_heap_limit=1024",
        "SELECT 1; SELECT 2; PRAGMA hard_heap_limit=1024",
    ] {
        let error = refused(&project, sql, "a PRAGMA after another statement")?;
        ensure_eq(&error.code, &rmcp::model::ErrorCode::INVALID_PARAMS, sql)?;
        ensure(
            error.message.contains("not PRAGMA"),
            format!("{sql}: unexpected message: {}", error.message),
        )?;
        ensure_eq(
            &fresh_pragma("hard_heap_limit")?,
            &before,
            format!("{sql} took effect"),
        )?;
    }
    Ok(())
}

#[test]
fn pragma_text_after_a_semicolon_inside_a_literal_is_not_refused() -> TestResult {
    let project = project()?;

    let response = ask(
        &project,
        "SELECT '; PRAGMA x' AS a, \"b; PRAGMA\" AS c FROM (SELECT 1 AS \"b; PRAGMA\") /* ; PRAGMA y */",
    )
    .check("a literal that mentions PRAGMA")?;

    ensure_eq(&response.rows.len(), &1, "response.rows.len() == 1")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("a")
            .check("response.rows[0][\"a\"]")?,
        &"; PRAGMA x",
        "response.rows[0][\"a\"] == \"; PRAGMA x\"",
    )?;
    Ok(())
}

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

#[test]
fn a_refused_vacuum_into_leaves_no_file() -> TestResult {
    let project = project()?;
    let target = project.path().join("vacuum-into.db");

    refused(
        &project,
        &format!("VACUUM INTO '{}'", target.display()),
        "a VACUUM INTO that writes a database file",
    )?;

    ensure(
        !target.exists(),
        "a refused VACUUM INTO must not create its target file",
    )?;
    Ok(())
}
