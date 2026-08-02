use super::*;
use rmcp::handler::server::wrapper::Parameters;
use std::fs;

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
    query(&Parameters(QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql: sql.to_owned(),
    }))
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

#[test]
fn joins_and_ctes_work_unlike_the_old_single_table_restriction() {
    let project = activated_project(&[("lib.rs", b"pub fn greet() {}\n")]);
    let response = ask(
        &project,
        "SELECT f.path, e.kind FROM files f JOIN evidence e ON e.file_id = f.id WHERE e.kind = 'Structure'",
    )
    .expect("join query succeeds");
    assert_eq!(response.rows.len(), 1);
    assert_eq!(response.rows[0]["path"], "lib.rs");

    let recursive = ask(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 5) SELECT x FROM cnt",
    )
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
    let huge = format!("SELECT {}", "1+".repeat(MAX_QUERY_LENGTH));
    let result = ask(&project, &huge);
    assert!(result.is_err());
}

#[test]
fn results_are_capped_and_truncation_is_reported() {
    let project = activated_project(&[]);
    let response = ask(
        &project,
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 600) SELECT x FROM cnt",
    )
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
