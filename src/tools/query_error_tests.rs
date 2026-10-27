//! The machine-readable code on `query` failures: SQLite's own compile and
//! run errors carry `sql_error` with a static remedy; refusals of the
//! statement's kind and failures raised on this side carry no code.
use super::*;
use crate::tools::test_support::activated_project;

fn ask(project: &tempfile::TempDir, sql: &str) -> ErrorData {
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
    .err()
    .expect("the statement fails")
}

#[test]
fn a_query_naming_an_unknown_table_carries_sql_error_and_its_remedy() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");

    let error = ask(&project, "SELECT nope FROM no_such_table");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error.message.contains("no such table: no_such_table"),
        "SQLite's own message is kept: {}",
        error.message
    );
    let data = error.data.expect("an SQL error carries data");
    assert_eq!(data["code"], "sql_error");
    assert_eq!(data["remedy"], ErrorCode::SqlError.remedy());
}

#[test]
fn a_query_with_a_syntax_error_carries_sql_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");

    let error = ask(&project, "SELECT FROM WHERE");

    assert!(
        error.message.contains("syntax error"),
        "SQLite's own message is kept: {}",
        error.message
    );
    let data = error.data.expect("a syntax error carries data");
    assert_eq!(data["code"], "sql_error");
}

#[test]
fn a_refused_pragma_statement_keeps_its_refusal_and_carries_no_code() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");

    let error = ask(&project, "PRAGMA user_version");

    assert!(
        error.message.starts_with("sql was refused"),
        "{}",
        error.message
    );
    assert_eq!(error.data, None, "a refusal is not an SQL error");
}

#[test]
fn a_value_over_the_per_value_cap_carries_no_sql_error_code() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let limits = ResourceLimits {
        max_query_value_bytes: 4,
        ..ResourceLimits::default()
    };

    let error = query_with_limits(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: "SELECT 'abcdefgh' AS text".to_owned(),
            cursor: None,
            legacy_paging: LegacyPaging::default(),
            page_bytes: None,
        }),
        &limits,
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .expect("an oversized value fails");

    assert!(error.message.contains("per-value cap"), "{}", error.message);
    assert_eq!(error.data, None, "the cap names its own fix; it is not SQL");
}
