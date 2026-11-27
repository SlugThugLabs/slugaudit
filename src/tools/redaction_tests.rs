//! Log-conciseness contract: tool-call logging paths emit compact operational
//! metadata (revision id, row/file counts, ids) rather than full SQL text,
//! finding prose, or indexed file contents. Error messages are logged once by
//! `server_runner::log_completion`; structured `ErrorData.data` carries only
//! static error codes and remedies.
//!
//! Each test captures at `DEBUG` and emits a control event to verify the
//! subscriber is active before checking that payload text was not logged.

use crate::store::test_capture::capture_at_level;
use crate::tools::test_support::activated_project;
use crate::tools::{FindingRequest, QueryRequest, ReportRequest};
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure};
use tracing::Level;

const SQL_SENTINEL: &str = "SENTINEL_SQL_9147";
const FINDING_TITLE_SENTINEL: &str = "SENTINEL_TITLE_9147";
const FINDING_DESC_SENTINEL: &str = "SENTINEL_DESC_9147";
const SOURCE_SENTINEL: &str = "SENTINEL_SOURCE_9147";
const CAPTURE_CONTROL: &str = "SENTINEL_CAPTURE_CONTROL_9147";

fn emit_capture_control() {
    tracing::debug!(control = CAPTURE_CONTROL, "log capture control");
}

fn assert_capture_is_live(logs: &str) -> TestResult {
    ensure(
        logs.contains(CAPTURE_CONTROL),
        format!("the DEBUG capture did not record the control event:\n{logs}"),
    )?;
    Ok(())
}

fn manager() -> crate::sync::SourceSyncManager {
    crate::sync::SourceSyncManager::default()
}

#[test]
fn query_logging_omits_sql_text() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let sql = format!("SELECT '{SQL_SENTINEL}' AS marker FROM files");
    let request = QueryRequest {
        path: Some(project.path().to_string_lossy().into_owned()),
        sql,
        cursor: None,
        legacy_paging: crate::tools::page::LegacyPaging::default(),
        page_bytes: None,
    };
    let (result, logs) = capture_at_level(
        || {
            emit_capture_control();
            crate::tools::query(
                &Parameters(request),
                &crate::progress::NoopProgressSink,
                &manager(),
            )
        },
        Level::DEBUG,
    )?;
    result.check("query succeeds")?;
    assert_capture_is_live(&logs)?;
    ensure(
        !logs.contains(SQL_SENTINEL),
        format!("query handler logged full SQL text instead of metadata:\n{logs}"),
    )?;
    Ok(())
}

#[test]
fn finding_logging_omits_finding_prose() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
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
            emit_capture_control();
            crate::tools::finding(
                &Parameters(request),
                &crate::progress::NoopProgressSink,
                &manager(),
            )
        },
        Level::DEBUG,
    )?;
    result.check("finding records")?;
    assert_capture_is_live(&logs)?;
    ensure(
        !logs.contains(FINDING_TITLE_SENTINEL) && !logs.contains(FINDING_DESC_SENTINEL),
        format!("finding handler logged finding prose instead of metadata:\n{logs}"),
    )?;
    Ok(())
}

#[test]
fn sync_and_report_logging_omits_source_content() -> TestResult {
    let project = activated_project(
        "lib.rs",
        format!("// {SOURCE_SENTINEL}\npub fn a() {{}}\n").as_bytes(),
    )?;
    let request = ReportRequest {
        path: project.path().to_string_lossy().into_owned(),
    };
    let (result, logs) = capture_at_level(
        || {
            emit_capture_control();
            crate::tools::report(
                &Parameters(request),
                &crate::progress::NoopProgressSink,
                &manager(),
            )
        },
        Level::DEBUG,
    )?;
    result.check("report builds")?;
    assert_capture_is_live(&logs)?;
    ensure(
        !logs.contains(SOURCE_SENTINEL),
        format!("sync/report logged indexed file contents:\n{logs}"),
    )?;
    Ok(())
}

#[test]
fn error_payload_carries_static_code_and_remedy_only() -> TestResult {
    let Err(error) =
        crate::tools::leads::mechanics::resolve_mechanic(Some("SENTINEL_MECHANIC_9147"))
    else {
        return Err("an unknown mechanic must fail".to_owned()).check("unexpected outcome");
    };
    ensure(
        error.message.contains("SENTINEL_MECHANIC_9147"),
        format!(
            "the message names what the caller asked for: {}",
            error.message
        ),
    )?;
    let payload = serde_json::to_string(&error.data).check("payload serializes")?;
    ensure(
        !payload.contains("SENTINEL_MECHANIC_9147"),
        format!("caller argument should stay in message, not structured payload: {payload}"),
    )?;
    ensure(
        payload.contains("unknown_mechanic"),
        format!("payload carries the machine-readable code: {payload}"),
    )?;
    Ok(())
}
