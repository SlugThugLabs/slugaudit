//! C13 redaction contract: no tool-call logging path may emit SQL text,
//! finding content, or source content into the logs.
//!
//! Every tool handler logs only structural metadata (revision id, counts,
//! file paths, ids) and never the payloads an AI actually cares about —
//! query SQL, finding titles/descriptions, or indexed source. This file
//! pins that contract with the same capture harness `store::test_capture`
//! provides, so a future `tracing::info!(sql = %sql)` or
//! `debug!(content = ...)` regression fails the suite instead of silently
//! shipping source code into the server's stderr log.

use crate::store::test_capture::capture_at_level;
use crate::tools::test_support::activated_project;
use crate::tools::{FindingRequest, QueryRequest, ReportRequest};
use rmcp::handler::server::wrapper::Parameters;
use tracing::Level;

/// Distinctive strings that must never appear in captured logs.
const SQL_SENTINEL: &str = "SENTINEL_SQL_9147";
const FINDING_TITLE_SENTINEL: &str = "SENTINEL_TITLE_9147";
const FINDING_DESC_SENTINEL: &str = "SENTINEL_DESC_9147";
const SOURCE_SENTINEL: &str = "SENTINEL_SOURCE_9147";

fn manager() -> crate::sync::SourceSyncManager {
    crate::sync::SourceSyncManager::default()
}

#[test]
fn query_logging_never_emits_sql_text() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let sql = format!("SELECT '{SQL_SENTINEL}' AS marker FROM files");
    let request = QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql,
        offset: 0,
    };
    let (result, logs) = capture_at_level(
        || {
            crate::tools::query(
                &Parameters(request),
                &crate::progress::NoopProgressSink,
                &manager(),
            )
        },
        Level::INFO,
    );
    result.expect("query succeeds");
    assert!(
        !logs.contains(SQL_SENTINEL),
        "the query tool leaked SQL text into logs:\n{logs}"
    );
}

#[test]
fn finding_logging_never_emits_finding_content() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: "lib.rs".to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "high".to_owned(),
        category: "correctness".to_owned(),
        title: FINDING_TITLE_SENTINEL.to_owned(),
        description: FINDING_DESC_SENTINEL.to_owned(),
    };
    let (result, logs) = capture_at_level(
        || {
            crate::tools::finding(
                &Parameters(request),
                &crate::progress::NoopProgressSink,
                &manager(),
            )
        },
        Level::INFO,
    );
    result.expect("finding records");
    assert!(
        !logs.contains(FINDING_TITLE_SENTINEL) && !logs.contains(FINDING_DESC_SENTINEL),
        "the finding tool leaked AI-authored judgment content into logs:\n{logs}"
    );
}

#[test]
fn sync_and_report_logging_never_emits_source_content() {
    // The source file's content is a sentinel; a full publish samples and
    // indexes it. If any sync/publish/report logging site emitted the
    // sampled content, the sentinel would appear in the captured logs.
    let project = activated_project(
        "lib.rs",
        format!("// {SOURCE_SENTINEL}\npub fn a() {{}}\n").as_bytes(),
    );
    let request = ReportRequest {
        path: project.path().to_string_lossy().into_owned(),
    };
    let (result, logs) = capture_at_level(
        || {
            crate::tools::report(
                &Parameters(request),
                &crate::progress::NoopProgressSink,
                &manager(),
            )
        },
        Level::INFO,
    );
    result.expect("report builds");
    assert!(
        !logs.contains(SOURCE_SENTINEL),
        "the sync/report path leaked source content into logs:\n{logs}"
    );
}
