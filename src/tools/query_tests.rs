use super::*;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;
use std::time::Duration;

fn activated_project(files: &[(&str, &[u8])]) -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    for (relative, content) in files {
        let path = project.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).check("create parent dirs")?;
        }
        fs::write(path, content).check("write fixture file")?;
    }
    Ok(project)
}

fn ask(project: &tempfile::TempDir, sql: &str) -> Result<QueryResponse, ErrorData> {
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

fn ask_with_limits(
    project: &tempfile::TempDir,
    sql: &str,
    limits: &ResourceLimits,
) -> Result<QueryResponse, ErrorData> {
    query_with_limits(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn a_null_byte_len_is_returned_as_null_not_zero() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;
    let response = ask(
        &project,
        "SELECT CAST(NULL AS INTEGER) AS byte_len, 0 AS empty_len FROM files",
    )
    .check("query succeeds")?;
    ensure(
        response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("byte_len")
            .check("response.rows[0][\"byte_len\"]")?
            .is_null(),
        "response.rows[0][\"byte_len\"].is_null()",
    )?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("empty_len")
            .check("response.rows[0][\"empty_len\"]")?,
        &0,
        "response.rows[0][\"empty_len\"] == 0",
    )?;
    Ok(())
}

#[test]
fn a_plain_select_returns_real_rows() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;
    let response = ask(&project, "SELECT path, language FROM files").check("query succeeds")?;
    ensure_eq(&response.rows.len(), &1, "response.rows.len() == 1")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("path")
            .check("response.rows[0][\"path\"]")?,
        &"lib.rs",
        "response.rows[0][\"path\"] == \"lib.rs\"",
    )?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("language")
            .check("response.rows[0][\"language\"]")?,
        &"rust",
        "response.rows[0][\"language\"] == \"rust\"",
    )?;
    ensure(response.page.complete, "response.page.complete")?;
    Ok(())
}

/// A trailing `-- line comment` must not swallow the `) LIMIT N` wrapper we
/// append around the user's SQL. The wrapper puts the user SQL on its own line
/// so the comment terminates before the closing paren.
#[test]
fn a_query_with_a_trailing_line_comment_still_executes() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;
    let response = ask(&project, "SELECT path FROM files -- get paths")
        .check("query with trailing comment succeeds")?;
    ensure_eq(&response.rows.len(), &1, "response.rows.len() == 1")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("path")
            .check("response.rows[0][\"path\"]")?,
        &"lib.rs",
        "response.rows[0][\"path\"] == \"lib.rs\"",
    )?;
    Ok(())
}

#[test]
fn joins_and_ctes_work_unlike_the_old_single_table_restriction() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn greet() {}\n")])?;
    let response = ask(
        &project,
        "SELECT f.path, e.kind FROM files f JOIN evidence e ON e.file_id = f.id WHERE e.kind = 'Structure'")
    .check("join query succeeds")?;
    ensure_eq(&response.rows.len(), &1, "response.rows.len() == 1")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("path")
            .check("response.rows[0][\"path\"]")?,
        &"lib.rs",
        "response.rows[0][\"path\"] == \"lib.rs\"",
    )?;

    let recursive = ask(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5) SELECT x FROM cnt")
    .check("recursive CTE query succeeds")?;
    ensure_eq(&recursive.rows.len(), &5, "recursive.rows.len() == 5")?;
    Ok(())
}

#[test]
fn a_write_attempt_fails_and_changes_nothing() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;
    let result = ask(&project, "DELETE FROM files");
    let err = result.check_err("write attempt on read-only query must fail")?;
    ensure_eq(
        &err.message.as_ref(),
        &"sql was refused: `query` accepts a single read-only SELECT/WITH statement",
        "err.message.as_ref() == \"sql was refused: `query` accepts a single read-only SELECT/WITH statement\"",
    )?;

    let after = ask(&project, "SELECT count(*) AS n FROM files").check("count still works")?;
    ensure_eq(
        &after
            .rows
            .first()
            .check("after.rows[0]")?
            .get("n")
            .check("after.rows[0][\"n\"]")?,
        &1,
        "after.rows[0][\"n\"] == 1",
    )?;
    Ok(())
}

/// A second database file is exactly what the wrapper exists to keep out of
/// reach. `ATTACH` and `DETACH` are both covered, with
/// the alias a refused `ATTACH` must never have created — a caller that only
/// checked for rows would not notice a silently readable `other.secrets`.
#[test]
fn an_attached_database_attempt_fails() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;
    let foreign_dir = tempfile::tempdir().check("foreign dir")?;
    let foreign_path = foreign_dir.path().join("foreign.db");
    let foreign = rusqlite::Connection::open(&foreign_path).check("create foreign db")?;
    foreign
        .execute_batch(
            "CREATE TABLE secrets (v TEXT); INSERT INTO secrets VALUES ('top-secret-value');",
        )
        .check("seed foreign db")?;
    drop(foreign);

    let result = ask(
        &project,
        &format!("ATTACH DATABASE '{}' AS other", foreign_path.display()),
    );
    ensure(
        result.is_err(),
        format!(
            "a foreign database must be refused, not attached: {:?}",
            result.map(|response| response.rows)
        ),
    )?;

    for sql in [
        "ATTACH DATABASE ':memory:' AS other",
        "DETACH DATABASE main",
        "SELECT v FROM other.secrets",
    ] {
        let result = ask(&project, sql);
        ensure(
            result.is_err(),
            format!(
                "`{sql}` must be refused, not answered with 0 rows: {:?}",
                result.map(|response| response.rows)
            ),
        )?;
    }

    // Re-read through the tool: the fixture row is still the only row.
    let after = ask(&project, "SELECT path FROM files").check("the index still reads")?;
    ensure_eq(
        &after.rows.len(),
        &1,
        format!("the index changed: {:?}", after.rows),
    )?;
    Ok(())
}

/// Multi-statement SQL can't be expressed by the subquery wrapper, but a
/// raw SQLite syntax error is confusing, so the caller is told plainly that
/// only one statement is allowed. SQLite's own parser (via rusqlite's
/// `MultipleStatement`) decides what counts as a second statement.
#[test]
fn more_than_one_statement_is_refused_with_the_single_statement_message() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;

    for sql in [
        "SELECT 1; SELECT 2",
        "SELECT 1; DROP TABLE files",
        "SELECT 'a;b'; SELECT 2",
        "SELECT 1 /* c */; SELECT 2",
        "SELECT '--x'; SELECT 2",
    ] {
        let error = ask(&project, sql).check_err("a second statement must be refused")?;
        ensure_eq(&error.code, &rmcp::model::ErrorCode::INVALID_PARAMS, sql)?;
        ensure(
            error
                .message
                .contains("only a single read-only statement is supported"),
            format!("{sql}: unexpected message: {}", error.message),
        )?;
    }

    let files = ask(&project, "SELECT path FROM files").check("the index is untouched")?;
    ensure_eq(&files.rows.len(), &1, format!("{:?}", files.rows))?;
    Ok(())
}

#[test]
fn a_single_statement_with_semicolons_or_trailing_trivia_is_accepted() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;

    for sql in [
        "SELECT 1;",
        "SELECT 1;\n",
        "SELECT 1; -- done",
        "SELECT 1 -- note",
        "SELECT ';' AS x",
        "SELECT 1 /* ; */",
        "SELECT 1;;",
        "SELECT 1; /* done */",
        "SELECT 1 AS \"a;b\"",
        "SELECT 'it''s; fine' AS x",
        "SELECT 1 AS [a;b]",
    ] {
        let response = ask(&project, sql).map_err(|e| e.message).check(sql)?;
        ensure_eq(&response.rows.len(), &1, sql)?;
        ensure(response.page.complete, sql)?;
    }
    let response = ask(&project, "SELECT ';' AS x").check("string semicolon is fine")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("x")
            .check("response.rows[0][\"x\"]")?,
        &";",
        "response.rows[0][\"x\"] == \";\"",
    )?;
    Ok(())
}

/// Multibyte text around a `;` must not confuse statement counting: a
/// multibyte string literal containing a semicolon, a multibyte quoted
/// identifier, and a semicolon inside a multibyte line comment are all one
/// statement, while a real separator after multibyte content still counts.
#[test]
fn multibyte_content_never_confuses_statement_counting() -> TestResult {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")])?;

    // A multibyte string literal with a semicolon inside is one statement.
    let response = ask(&project, "SELECT '日本語;です' AS x").check("multibyte string is fine")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("x")
            .check("response.rows[0][\"x\"]")?,
        &"日本語;です",
        "response.rows[0][\"x\"] == \"日本語;です\"",
    )?;

    // A multibyte quoted identifier with a semicolon is one statement.
    let response = ask(&project, "SELECT 1 AS \"値;列\"").check("multibyte quoted id is fine")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("値;列")
            .check("response.rows[0][\"値;列\"]")?,
        &1,
        "response.rows[0][\"値;列\"] == 1",
    )?;

    // A semicolon inside a multibyte line comment is one statement.
    let response =
        ask(&project, "SELECT 1 -- コメント ; コメント").check("multibyte comment is fine")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .as_object()
            .check("row is an object")?
            .len(),
        &1,
        "response.rows[0] .as_object() .check(\"row is an object\")? .len() == 1",
    )?;

    // A semicolon inside a multibyte block comment is one statement.
    let response = ask(&project, "SELECT 1 /* コメント ; コメント */ AS x")
        .check("multibyte block comment is fine")?;
    ensure_eq(
        &response
            .rows
            .first()
            .check("response.rows[0]")?
            .get("x")
            .check("response.rows[0][\"x\"]")?,
        &1,
        "response.rows[0][\"x\"] == 1",
    )?;

    // A real separator still counts even when multibyte text precedes it.
    let error = ask(&project, "SELECT '終わり'; SELECT 2")
        .check_err("a separator after multibyte content is still a separator")?;
    ensure(
        error.message.contains("single read-only statement"),
        "error.message.contains(\"single read-only statement\")",
    )?;
    Ok(())
}

#[test]
fn empty_sql_is_a_typed_error() -> TestResult {
    let project = activated_project(&[])?;
    let result = ask(&project, "   ");
    let err = result.check_err("empty sql must return ErrorData")?;
    ensure(
        err.message.contains("must not be empty"),
        format!("unexpected error message: {}", err.message),
    )?;
    Ok(())
}

/// An oversized statement is a contract error a caller can act on: it
/// carries `sql_too_large` and the knob that raises the cap, and — like
/// every coded payload — none of the agent's own SQL text, which stays in
/// the human message.
#[test]
fn oversized_sql_carries_its_code_and_never_the_sql_text() -> TestResult {
    let project = activated_project(&[])?;
    let sentinel = "SENTINEL_SQL_9147";
    let huge = format!(
        "SELECT '{sentinel}' {}",
        "1+".repeat(crate::model::ResourceLimits::default().max_query_sql_bytes)
    );
    let Err(error) = ask(&project, &huge) else {
        return Err("a statement over the byte cap must fail".to_owned())
            .check("unexpected outcome");
    };
    ensure(
        !error.message.contains(sentinel),
        format!(
            "the SQL text is not echoed in the message either: {}",
            error.message
        ),
    )?;
    let data = error
        .data
        .check("an oversized statement carries a payload")?;
    ensure_eq(
        &data.get("code").check("data[\"code\"]")?,
        &"sql_too_large",
        "data[\"code\"] == \"sql_too_large\"",
    )?;
    let remedy = data
        .get("remedy")
        .check("data[\"remedy\"]")?
        .as_str()
        .check("remedy is a string")?;
    ensure(
        remedy.contains("SLUGAUDIT_MAX_QUERY_SQL_BYTES"),
        format!("the remedy must name the knob: {remedy}"),
    )?;
    ensure(
        !serde_json::to_string(&data)
            .check("serializes")?
            .contains(sentinel),
        "!serde_json::to_string(&data) .check(\"serializes\")? .contains(sentinel)",
    )?;
    Ok(())
}

#[path = "query_page_tests.rs"]
mod query_page_tests;

#[path = "query_clip_tests.rs"]
mod query_clip_tests;

#[path = "query_budget_tests.rs"]
mod budget_tests;
