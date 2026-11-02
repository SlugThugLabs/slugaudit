//! End-to-end proof that every surface reports the same 1-based, inclusive
//! line numbers for one fixture: `leads`, `query` (evidence rows and
//! `line_range`), `structure`, and the stored `finding` range.
use super::*;
use crate::tools::finding::{FindingRequest, finding};
use crate::tools::query::{QueryRequest, query};
use crate::tools::structure::{StructureRequest, structure};
use slugaudit_test_check::{Check, TestResult};

/// The lines `cat -n` shows: `x.unwrap()` on 3, the TODO comment on 4,
/// `fn helper` on 6.
const FIXTURE: &str =
    "fn main() {\n    let x = Some(1);\n    x.unwrap();\n    // TODO: fix\n}\nfn helper() {}\n";

/// The 1-based line of the first fixture line containing `needle`, found by
/// scanning the text rather than trusting a hardcoded number.
fn scanned_line(needle: &str) -> TestResult<i64> {
    let index = FIXTURE
        .lines()
        .position(|line| line.contains(needle))
        .check("needle is in the fixture")?;
    Ok(i64::try_from(index).check("small index")? + 1)
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

#[test]
fn every_surface_reports_the_same_one_based_inclusive_lines() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(project.path().join("main.rs"), FIXTURE).check("write fixture")?;
    assert_eq!((scanned_line("unwrap")?, scanned_line("TODO")?), (3, 4));
    assert_eq!(scanned_line("fn helper")?, 6);

    // leads
    let unwrap = run(request(project.path(), Some("unwrap_calls")))?;
    let todo = run(request(project.path(), Some("comment_markers")))?;
    let unwrap = full_rows(&unwrap)?;
    let todo = full_rows(&todo)?;
    assert_eq!(unwrap.len(), 1);
    assert_eq!(unwrap[0].line, 3);
    assert_eq!(todo.len(), 1);
    assert_eq!(todo[0].line, 4);

    // query: evidence rows
    let call = sql_rows(
        &project,
        "SELECT start_line, end_line FROM evidence WHERE kind = 'Call' \
         AND json_extract(payload, '$.callee_name') = 'unwrap'",
    )?;
    assert_eq!(call.len(), 1);
    assert_eq!(
        (call[0]["start_line"].as_i64(), call[0]["end_line"].as_i64()),
        (Some(3), Some(3))
    );
    let comment = sql_rows(
        &project,
        "SELECT start_line, end_line FROM evidence WHERE kind = 'Comment' \
         AND json_extract(payload, '$.text') LIKE '%TODO%'",
    )?;
    assert_eq!(comment.len(), 1);
    assert_eq!(comment[0]["start_line"].as_i64(), Some(4));
    assert_eq!(comment[0]["end_line"].as_i64(), Some(4));

    // structure
    assert_eq!(structure_lines(&project, "fn main")?, (1, 5));
    assert_eq!(structure_lines(&project, "fn helper")?, (6, 6));

    // finding echo: the stored range is exactly the submitted one
    finding(
        &Parameters(FindingRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: "main.rs".to_owned(),
            line_start: 3,
            line_end: 4,
            severity: "low".to_owned(),
            category: "correctness".to_owned(),
            title: "unwrap before TODO".to_owned(),
            description: "Lines 3 to 4 of main.rs.".to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("finding succeeds")?;
    let findings_db = crate::tools::test_support::own_findings_path(&project)?;
    let connection = crate::store::open_findings(&findings_db).check("open findings db")?;
    let stored: (i64, i64) = connection
        .query_row("SELECT line_start, line_end FROM findings", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .check("read finding range")?;
    assert_eq!(stored, (3, 4));

    // line_range over the same range
    let ranged = sql_rows(
        &project,
        "SELECT line_range(content, 3, 4) AS r FROM files WHERE path = 'main.rs'",
    )?;
    assert_eq!(
        ranged[0]["r"].as_str(),
        Some("    x.unwrap();\n    // TODO: fix")
    );
    Ok(())
}
