use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;
use std::time::Duration;

fn activated_project(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    for (relative, content) in files {
        let path = project.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, content).expect("write fixture file");
    }
    project
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
fn a_null_byte_len_is_returned_as_null_not_zero() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let response = ask(
        &project,
        "SELECT CAST(NULL AS INTEGER) AS byte_len, 0 AS empty_len FROM files",
    )
    .expect("query succeeds");
    assert!(response.rows[0]["byte_len"].is_null());
    assert_eq!(response.rows[0]["empty_len"], 0);
}

#[test]
fn a_plain_select_returns_real_rows() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let response = ask(&project, "SELECT path, language FROM files").expect("query succeeds");
    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0]["path"], "lib.rs");
    assert_eq!(response.rows[0]["language"], "rust");
    assert!(response.page.complete);
}

/// A trailing `-- line comment` must not swallow the `) LIMIT N` wrapper we
/// append around the user's SQL. The fix puts the user SQL on its own line
/// so the comment terminates before the closing paren.
#[test]
fn a_query_with_a_trailing_line_comment_still_executes() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let response = ask(&project, "SELECT path FROM files -- get paths")
        .expect("query with trailing comment succeeds");
    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0]["path"], "lib.rs");
}

#[test]
fn joins_and_ctes_work_unlike_the_old_single_table_restriction() {
    let project = activated_project(&[("lib.rs", b"pub fn greet() {}\n")]);
    let response = ask(
        &project,
        "SELECT f.path, e.kind FROM files f JOIN evidence e ON e.file_id = f.id WHERE e.kind = 'Structure'")
    .expect("join query succeeds");
    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0]["path"], "lib.rs");

    let recursive = ask(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5) SELECT x FROM cnt")
    .expect("recursive CTE query succeeds");
    assert_eq!(recursive.rows.len(), 5);
}

#[test]
fn a_write_attempt_fails_and_changes_nothing() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let result = ask(&project, "DELETE FROM files");
    let err = result.expect_err("write attempt on read-only query must fail");
    assert!(
        err.message.to_lowercase().contains("syntax")
            || err.message.to_lowercase().contains("readonly")
            || err.message.to_lowercase().contains("not authorized"),
        "unexpected error message: {}",
        err.message
    );

    let after = ask(&project, "SELECT count(*) AS n FROM files").expect("count still works");
    assert_eq!(after.rows[0]["n"], 1);
}

/// A second database file is exactly what the wrapper and the connection's
/// authorizer exist to keep out of reach. Both directions of the same
/// authorizer decision are covered here (`ATTACH` and `DETACH`), together
/// with the alias a refused `ATTACH` must never have created — a caller
/// that only checked for rows would not notice a silently readable
/// `other.secrets`.
#[test]
fn an_attached_database_attempt_fails() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let foreign_dir = tempfile::tempdir().expect("foreign dir");
    let foreign_path = foreign_dir.path().join("foreign.db");
    let foreign = rusqlite::Connection::open(&foreign_path).expect("create foreign db");
    foreign
        .execute_batch(
            "CREATE TABLE secrets (v TEXT); INSERT INTO secrets VALUES ('top-secret-value');",
        )
        .expect("seed foreign db");
    drop(foreign);

    let result = ask(
        &project,
        &format!("ATTACH DATABASE '{}' AS other", foreign_path.display()),
    );
    assert!(
        result.is_err(),
        "a foreign database must be refused, not attached: {:?}",
        result.map(|response| response.rows)
    );

    for sql in [
        "ATTACH DATABASE ':memory:' AS other",
        "DETACH DATABASE main",
        "SELECT v FROM other.secrets",
    ] {
        let result = ask(&project, sql);
        assert!(
            result.is_err(),
            "`{sql}` must be refused, not answered with 0 rows: {:?}",
            result.map(|response| response.rows)
        );
    }

    // Re-read through the tool: the fixture row is still the only row.
    let after = ask(&project, "SELECT path FROM files").expect("the index still reads");
    assert_eq!(after.rows.len(), 1, "the index changed: {:?}", after.rows);
}

/// Multi-statement SQL can't be expressed by the subquery wrapper, but a
/// raw SQLite syntax error is confusing, so the caller is told plainly that
/// only one statement is allowed. SQLite's own parser (via rusqlite's
/// `MultipleStatement`) decides what counts as a second statement.
#[test]
fn more_than_one_statement_is_refused_with_the_single_statement_message() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);

    for sql in [
        "SELECT 1; SELECT 2",
        "SELECT 1; DROP TABLE files",
        "SELECT 'a;b'; SELECT 2",
        "SELECT 1 /* c */; SELECT 2",
        "SELECT '--x'; SELECT 2",
    ] {
        let error = ask(&project, sql).expect_err("a second statement must be refused");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{sql}");
        assert!(
            error
                .message
                .contains("only a single read-only statement is supported"),
            "{sql}: unexpected message: {}",
            error.message
        );
    }

    let files = ask(&project, "SELECT path FROM files").expect("the index is untouched");
    assert_eq!(files.rows.len(), 1, "{:?}", files.rows);
}

#[test]
fn a_single_statement_with_semicolons_or_trailing_trivia_is_accepted() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);

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
        let response = ask(&project, sql).unwrap_or_else(|e| panic!("{sql}: {}", e.message));
        assert_eq!(response.rows.len(), 1, "{sql}");
        assert!(response.page.complete, "{sql}");
    }
    let response = ask(&project, "SELECT ';' AS x").expect("string semicolon is fine");
    assert_eq!(response.rows[0]["x"], ";");
}

/// Multibyte text around a `;` must not confuse statement counting: a
/// multibyte string literal containing a semicolon, a multibyte quoted
/// identifier, and a semicolon inside a multibyte line comment are all one
/// statement, while a real separator after multibyte content still counts.
#[test]
fn multibyte_content_never_confuses_statement_counting() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);

    // A multibyte string literal with a semicolon inside is one statement.
    let response = ask(&project, "SELECT '日本語;です' AS x").expect("multibyte string is fine");
    assert_eq!(response.rows[0]["x"], "日本語;です");

    // A multibyte quoted identifier with a semicolon is one statement.
    let response = ask(&project, "SELECT 1 AS \"値;列\"").expect("multibyte quoted id is fine");
    assert_eq!(response.rows[0]["値;列"], 1);

    // A semicolon inside a multibyte line comment is one statement.
    let response =
        ask(&project, "SELECT 1 -- コメント ; コメント").expect("multibyte comment is fine");
    assert_eq!(
        response.rows[0]
            .as_object()
            .expect("row is an object")
            .len(),
        1
    );

    // A semicolon inside a multibyte block comment is one statement.
    let response = ask(&project, "SELECT 1 /* コメント ; コメント */ AS x")
        .expect("multibyte block comment is fine");
    assert_eq!(response.rows[0]["x"], 1);

    // A real separator still counts even when multibyte text precedes it.
    let error = ask(&project, "SELECT '終わり'; SELECT 2")
        .expect_err("a separator after multibyte content is still a separator");
    assert!(error.message.contains("single read-only statement"));
}

#[test]
fn empty_sql_is_a_typed_error() {
    let project = activated_project(&[]);
    let result = ask(&project, "   ");
    let err = result.expect_err("empty sql must return ErrorData");
    assert!(
        err.message.contains("must not be empty"),
        "unexpected error message: {}",
        err.message
    );
}

/// An oversized statement is a contract error a caller can act on: it
/// carries `sql_too_large` and the knob that raises the cap, and — like
/// every coded payload — none of the agent's own SQL text, which stays in
/// the human message.
#[test]
fn oversized_sql_carries_its_code_and_never_the_sql_text() {
    let project = activated_project(&[]);
    let sentinel = "SENTINEL_SQL_9147";
    let huge = format!(
        "SELECT '{sentinel}' {}",
        "1+".repeat(crate::model::ResourceLimits::default().max_query_sql_bytes)
    );
    let error = match ask(&project, &huge) {
        Err(error) => error,
        Ok(_) => panic!("a statement over the byte cap must fail"),
    };
    assert!(
        !error.message.contains(sentinel),
        "the SQL text is not echoed in the message either: {}",
        error.message
    );
    let data = error
        .data
        .expect("an oversized statement carries a payload");
    assert_eq!(data["code"], "sql_too_large");
    let remedy = data["remedy"].as_str().expect("remedy is a string");
    assert!(
        remedy.contains("SLUGAUDIT_MAX_QUERY_SQL_BYTES"),
        "the remedy must name the knob: {remedy}"
    );
    assert!(
        !serde_json::to_string(&data)
            .expect("serializes")
            .contains(sentinel)
    );
}

const COUNT_TO_900: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL \
    SELECT x + 1 FROM cnt WHERE x < 900) SELECT x FROM cnt";

#[test]
fn more_than_500_rows_come_back_in_one_page_when_the_byte_budget_allows() {
    let project = activated_project(&[]);

    let response = ask(&project, COUNT_TO_900).expect("recursive CTE query succeeds");

    assert_eq!(response.rows.len(), 900);
    assert!(response.page.complete);
    assert_eq!(response.page.cursor, None);
}

#[test]
fn a_page_is_cut_by_the_byte_budget_and_carries_a_cursor() {
    let project = activated_project(&[]);
    let limits = ResourceLimits {
        max_query_response_bytes: 200,
        ..ResourceLimits::default()
    };

    let first = ask_with_limits(&project, COUNT_TO_900, &limits).expect("first page");

    assert!(!first.page.complete);
    assert!(first.rows.len() > 1 && first.rows.len() < 900);
    assert!(first.page.cursor.is_some());
    assert_eq!(first.rows[0]["x"], 1);
}

#[test]
fn an_agent_limit_and_offset_still_work_inside_the_subquery() {
    let project = activated_project(&[]);

    let response = ask(&project, &format!("{COUNT_TO_900} LIMIT 3 OFFSET 10;"))
        .expect("the agent's own LIMIT/OFFSET is honored");

    let xs: Vec<_> = response.rows.iter().map(|r| r["x"].clone()).collect();
    assert_eq!(xs, [11, 12, 13]);
    assert!(response.page.complete);
}

#[test]
fn query_on_an_inactive_project_is_a_typed_error_not_a_panic() {
    let project = tempfile::tempdir().expect("project dir");
    let error = match ask(&project, "SELECT 1") {
        Err(error) => error,
        Ok(_) => panic!("an inactive project must fail"),
    };
    // The disabled state is one instruction, not one opaque failure per
    // tool: the prose stays, and the machine-readable half says *which*
    // state this is and which call leaves it.
    assert!(
        error.message.contains("not enabled"),
        "the existing wording stays: {}",
        error.message
    );
    let data = error.data.expect("the disabled state carries a payload");
    assert_eq!(data["code"], "project_not_enabled");
    let remedy = data["remedy"].as_str().expect("remedy is a string");
    assert!(remedy.contains("project_control"), "{remedy}");
    assert!(remedy.contains("\"action\":\"on\""), "{remedy}");
}

#[test]
fn an_oversized_text_or_blob_value_is_rejected_before_being_expanded() {
    let project = activated_project(&[]);
    let limits = ResourceLimits {
        max_query_value_bytes: 1024,
        ..ResourceLimits::default()
    };
    // zeroblob/printf avoid embedding a huge literal in the test source:
    // SQLite materializes the oversized value, and the per-value cap must
    // reject it before it is cloned or hex-expanded into JSON.
    for sql in [
        "SELECT zeroblob(4096) AS huge",
        "SELECT printf('%.*c', 4096, 'x') AS huge",
    ] {
        let error =
            ask_with_limits(&project, sql, &limits).expect_err("oversized value is rejected");
        assert!(
            error.message.contains("per-value cap"),
            "unexpected message for {sql}: {}",
            error.message
        );
    }
}

#[test]
fn a_pathological_query_is_aborted_by_the_step_budget() {
    let project = activated_project(&[]);
    let limits = ResourceLimits {
        max_query_vm_steps: 10,
        ..ResourceLimits::default()
    };
    let result = ask_with_limits(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5000) SELECT x FROM cnt",
        &limits,
    );
    let error = result.expect_err("runaway query is aborted, not run to completion");
    assert!(
        error.message.contains("step budget"),
        "unexpected message: {}",
        error.message
    );
}

#[test]
fn a_pathological_query_is_aborted_by_the_wall_clock_budget() {
    let project = activated_project(&[]);
    // Effectively zero: by the time the progress handler first fires (every
    // 1000 VM steps), any positive elapsed time trips this deadline, while
    // the step budget stays at its generous default so it is not what
    // catches the query.
    let limits = ResourceLimits {
        max_query_wall_clock: Duration::from_nanos(1),
        ..ResourceLimits::default()
    };
    let manager = crate::sync::SourceSyncManager::default();
    let result = query_with_limits(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5000) SELECT x FROM cnt".to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        &limits,
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .map(|Json(response)| response);
    let error = result.expect_err("runaway query is aborted, not run to completion");
    assert!(
        error.message.contains("time budget"),
        "unexpected message: {}",
        error.message
    );
    let store = manager
        .result_stores()
        .current()
        .expect("the store opened for the attempt");
    let stats = store.stats();
    assert_eq!(
        (
            stats.active.results,
            stats.idle.results,
            stats.delivered.results
        ),
        (0, 0, 0),
        "no partial result is left behind"
    );
}

#[test]
fn results_exceeding_response_bytes_are_cut_into_pages() {
    let project = activated_project(&[]);
    let limits = ResourceLimits {
        max_query_response_bytes: 50,
        ..ResourceLimits::default()
    };
    let response = ask_with_limits(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 50) SELECT printf('long_string_value_%d', x) AS s FROM cnt",
        &limits,
    )
    .expect("query succeeds");
    assert!(!response.page.complete);
    assert!(!response.rows.is_empty());
    assert!(response.rows.len() < 50);
}

#[path = "query_page_tests.rs"]
mod query_page_tests;

#[path = "query_clip_tests.rs"]
mod query_clip_tests;
