//! End-to-end proof that every surface reports the same 1-based, inclusive
//! line numbers for one fixture: `leads`, `query` (evidence rows and
//! `line_range`), `structure`, and the stored `finding` range.
use super::*;
use crate::tools::finding::{FindingRequest, finding};
use crate::tools::query::{QueryRequest, query};
use crate::tools::structure::{StructureRequest, structure};
use slugaudit_test_check::{Check, TestResult, ensure_eq};

/// The lines `cat -n` shows: `assert!` on 3, the TODO comment on 4,
/// `fn helper` on 6.
const FIXTURE: &str =
    "fn main() {\n    let x = 1;\n    assert!(x > 0);\n    // TODO: fix\n}\nfn helper() {}\n";

/// The 1-based line of the first fixture line containing `needle`, found by
/// scanning the text rather than trusting a hardcoded number.
fn scanned_line(needle: &str) -> TestResult<i64> {
    let index = FIXTURE
        .lines()
        .position(|line| line.contains(needle))
        .check("needle is in the fixture")?;
    let line = index
        .checked_add(1)
        .check("line number does not overflow")?;
    i64::try_from(line).check("small line number")
}

fn sql_rows(project: &tempfile::TempDir, sql: &str) -> TestResult<Vec<serde_json::Value>> {
    Ok(query(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: crate::tools::page::LegacyPaging::default(),
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("query succeeds")?
    .0
    .rows)
}

fn structure_lines(project: &tempfile::TempDir, name: &str) -> TestResult<(u32, u32)> {
    let response = structure(
        &Parameters(StructureRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: Some("main.rs".to_owned()),
            query: "(function_item) @fn".to_owned(),
            ..Default::default()
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("structure succeeds")?
    .0;
    let found = response
        .matches
        .iter()
        .find(|m| m.text.contains(name))
        .check("function matched")?;
    Ok((found.start_line, found.end_line))
}

/// The integer under `key` of one query row, `None` when it is not an integer.
fn json_line(row: &serde_json::Value, key: &str) -> TestResult<Option<i64>> {
    Ok(row.get(key).check("row has the line column")?.as_i64())
}

/// Finding echo: the stored range is exactly the submitted one.
fn stored_finding_range_is_the_submitted_one(project: &tempfile::TempDir) -> TestResult {
    finding(
        &Parameters(FindingRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: "main.rs".to_owned(),
            line_start: 3,
            line_end: 4,
            severity: "low".to_owned(),
            category: "correctness".to_owned(),
            title: "assert before TODO".to_owned(),
            description: "Lines 3 to 4 of main.rs.".to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("finding succeeds")?;
    let findings_db = crate::tools::test_support::own_findings_path(project)?;
    let connection = crate::store::open_findings(&findings_db).check("open findings db")?;
    let stored: (i64, i64) = connection
        .query_row("SELECT line_start, line_end FROM findings", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .check("read finding range")?;
    ensure_eq(&stored, &(3, 4), "stored == (3, 4)")?;
    Ok(())
}

#[test]
fn every_surface_reports_the_same_one_based_inclusive_lines() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(project.path().join("main.rs"), FIXTURE).check("write fixture")?;
    ensure_eq(
        &(scanned_line("assert")?, scanned_line("TODO")?),
        &(3, 4),
        "(scanned_line(\"assert\")?, scanned_line(\"TODO\")?) == (3, 4)",
    )?;
    ensure_eq(
        &scanned_line("fn helper")?,
        &6,
        "scanned_line(\"fn helper\")? == 6",
    )?;

    // leads
    let asserted = run(request(project.path(), Some("assert_calls")))?;
    let todo = run(request(project.path(), Some("comment_markers")))?;
    let asserted = full_rows(&asserted)?;
    let todo = full_rows(&todo)?;
    ensure_eq(&asserted.len(), &1, "asserted.len() == 1")?;
    ensure_eq(
        &asserted.first().check("asserted[0]")?.line,
        &3,
        "asserted[0].line == 3",
    )?;
    ensure_eq(&todo.len(), &1, "todo.len() == 1")?;
    ensure_eq(
        &todo.first().check("todo[0]")?.line,
        &4,
        "todo[0].line == 4",
    )?;

    // query: evidence rows
    let call = sql_rows(
        &project,
        "SELECT start_line, end_line FROM evidence WHERE kind = 'Call' \
         AND json_extract(payload, '$.callee_name') = 'assert'",
    )?;
    ensure_eq(&call.len(), &1, "call.len() == 1")?;
    let call_row = call.first().check("call[0]")?;
    ensure_eq(
        &(
            json_line(call_row, "start_line")?,
            json_line(call_row, "end_line")?,
        ),
        &(Some(3), Some(3)),
        "(call[0][\"start_line\"].as_i64(), call[0][\"end_line\"].as_i64()) == (Some(3), Some(3))",
    )?;
    let comment = sql_rows(
        &project,
        "SELECT start_line, end_line FROM evidence WHERE kind = 'Comment' \
         AND json_extract(payload, '$.text') LIKE '%TODO%'",
    )?;
    ensure_eq(&comment.len(), &1, "comment.len() == 1")?;
    let comment_row = comment.first().check("comment[0]")?;
    ensure_eq(
        &json_line(comment_row, "start_line")?,
        &Some(4),
        "comment[0][\"start_line\"].as_i64() == Some(4)",
    )?;
    ensure_eq(
        &json_line(comment_row, "end_line")?,
        &Some(4),
        "comment[0][\"end_line\"].as_i64() == Some(4)",
    )?;

    // structure
    ensure_eq(
        &structure_lines(&project, "fn main")?,
        &(1, 5),
        "structure_lines(&project, \"fn main\")? == (1, 5)",
    )?;
    ensure_eq(
        &structure_lines(&project, "fn helper")?,
        &(6, 6),
        "structure_lines(&project, \"fn helper\")? == (6, 6)",
    )?;

    stored_finding_range_is_the_submitted_one(&project)?;

    // line_range over the same range
    let ranged = sql_rows(
        &project,
        "SELECT line_range(content, 3, 4) AS r FROM files WHERE path = 'main.rs'",
    )?;
    ensure_eq(
        &ranged
            .first()
            .check("ranged[0][\"r\"]")?
            .get("r")
            .check("ranged[0][\"r\"]")?
            .as_str(),
        &Some("    assert!(x > 0);\n    // TODO: fix"),
        "ranged[0][\"r\"].as_str() == Some(\" assert!(x > 0);\\n // TODO: fix\")",
    )?;
    Ok(())
}
