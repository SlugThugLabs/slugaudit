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
fn an_inactive_project_is_a_typed_error_not_a_panic() {
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
