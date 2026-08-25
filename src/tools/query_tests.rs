// slugaudit-line-exception: approved-by=agent; reason=one test per SQL safety property; splitting would obscure the read-only boundary they collectively pin
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
            path: project.path().to_string_lossy().into_owned(),
            sql: sql.to_owned(),
            offset: 0,
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
            path: project.path().to_string_lossy().into_owned(),
            sql: sql.to_owned(),
            offset: 0,
        }),
        limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
}

#[test]
fn a_plain_select_returns_real_rows() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let response = ask(&project, "SELECT path, language FROM files").expect("query succeeds");
    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0]["path"], "lib.rs");
    assert_eq!(response.rows[0]["language"], "rust");
    assert!(!response.truncated);
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
    assert!(result.is_err());

    let after = ask(&project, "SELECT count(*) AS n FROM files").expect("count still works");
    assert_eq!(after.rows[0]["n"], 1);
}

#[test]
fn an_attached_database_attempt_fails() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let result = ask(&project, "ATTACH DATABASE ':memory:' AS other");
    assert!(result.is_err());
}

/// Multi-statement SQL can't be expressed by the subquery wrapper, but a
/// raw SQLite "near \";\": syntax error" is confusing — the caller should
/// be told plainly that only one statement is allowed. The separator
/// scanner must not misfire on `;` inside string literals, quoted
/// identifiers, or comments.
#[test]
fn multi_statement_sql_gets_a_friendly_typed_error() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);
    let error = ask(&project, "SELECT 1; SELECT 2")
        .expect_err("multi-statement sql is rejected with a typed error");
    assert!(
        error.message.contains("more than one statement"),
        "unexpected message: {}",
        error.message
    );

    // A semicolon inside a string literal is one statement and must work.
    let response = ask(&project, "SELECT ';' AS x").expect("string semicolon is fine");
    assert_eq!(response.rows[0]["x"], ";");

    // A semicolon inside a quoted identifier is one statement.
    let response = ask(&project, "SELECT 1 AS \"a;b\"").expect("quoted identifier is fine");
    assert_eq!(response.rows[0]["a;b"], 1);

    // A semicolon inside a comment is one statement.
    let response = ask(&project, "SELECT 1 -- ; trailing").expect("comment semicolon is fine");
    assert_eq!(response.rows[0].as_object().unwrap().len(), 1);

    // A single trailing semicolon is stripped, not treated as a separator.
    ask(&project, "SELECT 1;").expect("trailing semicolon is fine");

    // Escaped quotes must not confuse the scanner ('' inside a string).
    ask(&project, "SELECT 'it''s; fine' AS x").expect("escaped quote is fine");

    // A semicolon inside a block comment is one statement.
    let response = ask(&project, "SELECT 1 /* ; */ AS x").expect("block comment is fine");
    assert_eq!(response.rows[0]["x"], 1);

    // A semicolon inside backtick/bracket identifiers is one statement.
    let response = ask(&project, "SELECT 1 AS `a;b`").expect("backtick identifier is fine");
    assert_eq!(response.rows[0]["a;b"], 1);
    let response = ask(&project, "SELECT 1 AS [a;b]").expect("bracket identifier is fine");
    assert_eq!(response.rows[0]["a;b"], 1);

    // A separator after a closed block comment is still detected.
    let error = ask(&project, "SELECT 1 /* c */; SELECT 2")
        .expect_err("semicolon after a block comment is a real separator");
    assert!(error.message.contains("more than one statement"));

    // Comment markers inside a literal are ordinary characters: the old
    // scanner checked comments *before* string state, so `--` inside a
    // string skipped to end-of-line and hid the real separator, replacing
    // the friendly error with SQLite's raw `near \";\"` error.
    for sql in [
        "SELECT '--x'; SELECT 2",
        "SELECT '/*' AS x; SELECT 2",
        "SELECT 1 AS \"a--b\"; SELECT 2",
    ] {
        let error = ask(&project, sql)
            .expect_err("a comment marker inside a literal must not hide the real separator");
        assert!(error.message.contains("more than one statement"));
    }
    for sql in ["SELECT '--x' AS x", "SELECT '/*' AS x", "SELECT 1 AS \"a--b\""] {
        ask(&project, sql).expect("a single statement with a comment marker in a literal works");
    }
}

/// Scanner-level pinning of the comment-vs-string ordering: `--`/`/*`
/// inside a literal or quoted identifier are ordinary characters, and the
/// offsets are the first *top-level* `;`. (End-to-end behavior is covered
/// by `multi_statement_sql_gets_a_friendly_typed_error`.)
#[test]
fn separator_scanner_ignores_comment_markers_inside_literals() {
    assert_eq!(first_statement_separator("SELECT '--' AS x; SELECT 2"), Some(16));
    assert_eq!(first_statement_separator("SELECT '--x'; SELECT 2"), Some(12));
    assert_eq!(first_statement_separator("SELECT '/*' AS x; SELECT 2"), Some(16));
    assert_eq!(first_statement_separator("SELECT 'a' || '/*'; SELECT 2"), Some(18));
    assert_eq!(first_statement_separator("SELECT 1 AS \"a--b\"; SELECT 2"), Some(18));
    assert_eq!(first_statement_separator("SELECT 1 AS `a--b`; SELECT 2"), Some(18));
    assert_eq!(first_statement_separator("SELECT 1 AS [a--b]; SELECT 2"), Some(18));
    assert_eq!(first_statement_separator("SELECT ';--' AS x; SELECT 2"), Some(17));
    assert_eq!(first_statement_separator("SELECT '--' AS x"), None);
    assert_eq!(first_statement_separator("SELECT 1 AS \"a--b\""), None);
    assert_eq!(first_statement_separator("SELECT 1 -- ;\nSELECT 2"), None);
    assert_eq!(first_statement_separator("SELECT 1 /* ; */; SELECT 2"), Some(16));
}

/// The separator scanner walks bytes (`sql.as_bytes()`), which is correct
/// for UTF-8 by construction: no multibyte character's encoding ever
/// contains an ASCII byte (leading bytes are ≥ 0xC0, continuation bytes are
/// 0x80–0xBF), so the ASCII delimiters the scanner looks for (`;`, `'`,
/// `"`, `-`, `/`, `*`, backtick, bracket) can never appear inside a
/// multibyte character. This test pins that property end-to-end: a
/// multibyte string literal containing a semicolon, a multibyte quoted
/// identifier, and a semicolon inside a multibyte line comment must all be
/// treated as one statement.
#[test]
fn multibyte_content_never_confuses_the_statement_separator_scanner() {
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
    assert_eq!(response.rows[0].as_object().unwrap().len(), 1);

    // A semicolon inside a multibyte block comment is one statement.
    let response = ask(&project, "SELECT 1 /* コメント ; コメント */ AS x")
        .expect("multibyte block comment is fine");
    assert_eq!(response.rows[0]["x"], 1);

    // A real separator still counts even when multibyte text precedes it.
    let error = ask(&project, "SELECT '終わり'; SELECT 2")
        .expect_err("a separator after multibyte content is still a separator");
    assert!(error.message.contains("more than one statement"));
}

#[test]
fn empty_sql_is_a_typed_error() {
    let project = activated_project(&[]);
    let result = ask(&project, "   ");
    assert!(result.is_err());
}

#[test]
fn oversized_sql_is_a_typed_error() {
    let project = activated_project(&[]);
    let huge = format!(
        "SELECT {}",
        "1+".repeat(crate::model::ResourceLimits::default().max_query_sql_bytes)
    );
    let result = ask(&project, &huge);
    assert!(result.is_err());
}

#[test]
fn results_are_capped_and_truncation_is_reported() {
    let project = activated_project(&[]);
    let response = ask(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 600) SELECT x FROM cnt")
    .expect("recursive CTE query succeeds");
    assert_eq!(response.rows.len(), MAX_ROWS);
    assert!(response.truncated);
}

#[test]
fn query_on_an_inactive_project_is_a_typed_error_not_a_panic() {
    let project = tempfile::tempdir().expect("project dir");
    let result = ask(&project, "SELECT 1");
    assert!(result.is_err());
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
fn full_response_framing_is_counted_toward_the_byte_cap() {
    let project = activated_project(&[]);
    // Small enough that many individually-tiny rows still overflow once the
    // outer QueryResponse struct and array framing are counted.
    let limits = ResourceLimits {
        max_query_response_bytes: 300,
        ..ResourceLimits::default()
    };
    let response = ask_with_limits(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 100) SELECT x FROM cnt",
        &limits)
    .expect("query succeeds even though it must drop rows to fit");
    assert!(response.truncated);
    let encoded = serde_json::to_vec(&response).expect("response serializes");
    assert!(
        encoded.len() <= limits.max_query_response_bytes,
        "encoded response ({} bytes) exceeds the {}-byte cap",
        encoded.len(),
        limits.max_query_response_bytes
    );
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

/// C9: paging contract — `next_offset` is only valid while the revision
/// id does not change. A source edit between pages changes the revision,
/// and the response's `revision_id` lets the AI detect that boundary
/// (rows may shift order across a publish, so blindly continuing to page
/// could skip or duplicate rows).
#[test]
fn paging_across_a_revision_change_is_detectable_via_revision_id() {
    let project = activated_project(&[("lib.rs", b"pub fn a() {}\n")]);

    // Generate more than MAX_ROWS rows so paging is real (offset + next).
    let sql = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL \
               SELECT x + 1 FROM cnt WHERE x < 900) SELECT x FROM cnt";
    let page_one = ask(&project, sql).expect("page one succeeds");
    assert!(page_one.truncated);
    assert_eq!(page_one.rows.len(), MAX_ROWS);
    let next_offset = page_one.next_offset.expect("truncated query has next");
    assert_eq!(next_offset, MAX_ROWS);

    let page_two = ask(&project, sql).expect("page two succeeds");
    assert_eq!(
        page_two.revision_id, page_one.revision_id,
        "no source change: same revision, so paging stays valid"
    );

    // A source edit lands a new revision. The next page reports the new
    // revision id, which is exactly the boundary signal the contract
    // promises — the AI restarts paging rather than trusting next_offset.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() {}\npub fn b() {}\n",
    )
    .expect("modify fixture");
    let page_three = ask(&project, sql).expect("page three succeeds");
    assert_ne!(
        page_three.revision_id, page_one.revision_id,
        "a source change must produce a new revision, invalidating old offsets"
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
    let result = ask_with_limits(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5000) SELECT x FROM cnt",
        &limits,
    );
    let error = result.expect_err("runaway query is aborted, not run to completion");
    assert!(
        error.message.contains("time budget"),
        "unexpected message: {}",
        error.message
    );
}
