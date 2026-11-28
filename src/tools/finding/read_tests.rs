use super::*;
use crate::store;
use crate::sync;
use crate::tools::context::ensure_synced_no_progress;
use crate::tools::test_support::activated_project;
use crate::tools::{LegacyPaging, MAX_PATH_BYTES};
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

pub(super) fn request(path: &str, file: Option<&str>) -> FindingReadRequest {
    FindingReadRequest {
        path: Some(path.to_owned()),
        file: file.map(str::to_owned),
        cursor: None,
        page_bytes: None,
        legacy_paging: LegacyPaging::default(),
    }
}

pub(super) fn cursor_request(token: &str) -> FindingReadRequest {
    FindingReadRequest {
        path: None,
        file: None,
        cursor: Some(token.to_owned()),
        page_bytes: None,
        legacy_paging: LegacyPaging::default(),
    }
}

pub(super) fn write_finding(
    project: &tempfile::TempDir,
    file: &str,
    title: &str,
    manager: &sync::SourceSyncManager,
) -> TestResult<crate::tools::FindingResponse> {
    write_finding_with_description(project, file, title, "d", manager)
}

pub(super) fn write_finding_with_description(
    project: &tempfile::TempDir,
    file: &str,
    title: &str,
    description: &str,
    manager: &sync::SourceSyncManager,
) -> TestResult<crate::tools::FindingResponse> {
    Ok(crate::tools::finding(
        &Parameters(crate::tools::FindingRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: file.to_owned(),
            line_start: 1,
            line_end: 1,
            severity: "low".to_owned(),
            category: "test".to_owned(),
            title: title.to_owned(),
            description: description.to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("write finding")?
    .0)
}

/// Sets the global-config page budget `resolve_limits` reads, so a test can
/// set one without an env var (the environment is read once per process).
/// Keep the returned guard alive for the test.
pub(super) fn write_default_page_bytes(
    page_bytes: usize,
) -> TestResult<crate::tools::test_support::GlobalConfigGuard> {
    crate::tools::test_support::use_global_config(&format!(
        "{{\"default_page_bytes\": {page_bytes}}}"
    ))
}

pub(super) fn findings_db_path(project: &tempfile::TempDir) -> TestResult<std::path::PathBuf> {
    crate::tools::test_support::own_findings_path(project)
}

/// How many rows this process's findings file holds, read directly.
pub(super) fn stored_finding_count(project: &tempfile::TempDir) -> TestResult<i64> {
    store::open_findings_existing(&findings_db_path(project)?)
        .check("open findings db")?
        .check("the findings file exists")?
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .check("count findings")
}

#[test]
fn an_empty_session_returns_no_findings() -> TestResult {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
    let manager = sync::SourceSyncManager::default();

    let response = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("finding_read")?;
    ensure(
        response.0.findings.is_empty(),
        "response.0.findings.is_empty()",
    )?;
    Ok(())
}

#[test]
fn finding_persists_and_is_readable() -> TestResult {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
    let manager = sync::SourceSyncManager::default();

    let finding = write_finding(&project, "lib.rs", "test finding", &manager)?;
    ensure(finding.id > 0, "finding.id > 0")?;

    ensure_eq(
        &stored_finding_count(&project)?,
        &1,
        "stored_finding_count(&project)? == 1",
    )?;

    let response = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("finding_read")?;
    ensure_eq(
        &response.0.findings.len(),
        &1,
        "response.0.findings.len() == 1",
    )?;
    ensure_eq(
        &response
            .0
            .findings
            .first()
            .check("response.0.findings[0]")?
            .title,
        &"test finding",
        "response.0.findings[0].title == \"test finding\"",
    )?;
    Ok(())
}

#[test]
fn file_filter_returns_only_that_file() -> TestResult {
    let project = activated_project("a.rs", b"fn a() {}\n")?;
    std::fs::write(project.path().join("b.rs"), b"fn b() {}\n").check("b.rs")?;
    let manager = sync::SourceSyncManager::default();

    // Index b.rs so the finding tool can reference it.
    ensure_synced_no_progress(&project.path().to_string_lossy(), &manager).check("initial sync")?;

    write_finding(&project, "a.rs", "a finding", &manager)?;
    write_finding(&project, "b.rs", "b finding", &manager)?;

    ensure_eq(
        &stored_finding_count(&project)?,
        &2,
        "stored_finding_count(&project)? == 2",
    )?;

    let filtered = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), Some("a.rs"))),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("filtered")?;
    ensure_eq(
        &filtered.0.findings.len(),
        &1,
        "filtered.0.findings.len() == 1",
    )?;
    ensure_eq(
        &filtered
            .0
            .findings
            .first()
            .check("filtered.0.findings[0]")?
            .path,
        &"a.rs",
        "filtered.0.findings[0].path == \"a.rs\"",
    )?;
    Ok(())
}

#[test]
fn stale_findings_are_returned_with_stale_status() -> TestResult {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
    let manager = sync::SourceSyncManager::default();

    write_finding(&project, "lib.rs", "about to go stale", &manager)?;

    let before = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("before")?;
    ensure_eq(
        &before
            .0
            .findings
            .first()
            .check("before.0.findings[0]")?
            .status,
        &"current",
        "before.0.findings[0].status == \"current\"",
    )?;

    std::fs::write(project.path().join("lib.rs"), b"fn main() { changed(); }\n").check("modify")?;
    ensure_synced_no_progress(&project.path().to_string_lossy(), &manager).check("re-sync")?;

    let after = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("after")?;
    ensure_eq(
        &after
            .0
            .findings
            .first()
            .check("after.0.findings[0]")?
            .status,
        &"stale",
        "after.0.findings[0].status == \"stale\"",
    )?;
    Ok(())
}

#[test]
fn an_oversized_read_path_or_file_is_a_typed_error() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let base = directory.path().to_string_lossy().into_owned();

    let oversized_path = FindingReadRequest {
        path: Some("x".repeat(MAX_PATH_BYTES + 1)),
        ..request("", None)
    };
    let Err(path_error) = finding_read(
        &Parameters(oversized_path),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    ) else {
        return Err("an oversized path must be refused before resolution".to_owned())
            .check("unexpected outcome");
    };
    ensure_eq(
        &path_error.message,
        &"path exceeds 32768 bytes",
        "path_error.message == \"path exceeds 32768 bytes\"",
    )?;
    ensure_eq(
        &path_error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "path_error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;

    let oversized_file = FindingReadRequest {
        file: Some("x".repeat(MAX_FILE_BYTES + 1)),
        ..request(&base, None)
    };
    let Err(file_error) = finding_read(
        &Parameters(oversized_file),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    ) else {
        return Err("an oversized file filter must be refused".to_owned())
            .check("unexpected outcome");
    };
    ensure_eq(
        &file_error.message,
        &"file exceeds 8192 bytes",
        "file_error.message == \"file exceeds 8192 bytes\"",
    )?;
    ensure_eq(
        &file_error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "file_error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    Ok(())
}

/// A findings file that is not a database is an internal error from the
/// read, never an empty session.
#[test]
fn an_unreadable_findings_database_is_an_internal_error() -> TestResult {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
    let manager = sync::SourceSyncManager::default();
    ensure_synced_no_progress(&project.path().to_string_lossy(), &manager).check("first sync")?;
    std::fs::write(
        findings_db_path(&project)?,
        b"this is not a sqlite database",
    )
    .check("corrupt the findings file")?;

    let Err(error) = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    ) else {
        return Err("a corrupt findings database must fail the read".to_owned())
            .check("unexpected outcome");
    };

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure(
        error.message.contains("the session findings read"),
        format!("{}", error.message),
    )?;
    Ok(())
}

/// A findings row whose column has the wrong type cannot decode: an
/// internal error naming the column, not a skipped row.
#[test]
fn an_undecodable_finding_row_is_an_internal_error() -> TestResult {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
    let manager = sync::SourceSyncManager::default();
    write_finding(&project, "lib.rs", "decodable", &manager)?;
    store::open_findings(&findings_db_path(&project)?)
        .check("open findings db")?
        .execute("UPDATE findings SET path = x'ff'", [])
        .check("store a blob where text belongs")?;

    let Err(error) = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    ) else {
        return Err("an undecodable row must fail the read".to_owned()).check("unexpected outcome");
    };

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure(
        error.message.contains("reading a session finding")
            && error.message.contains("Invalid column type"),
        format!("{}", error.message),
    )?;
    Ok(())
}

/// A description too large for the budget is clipped on a character
/// boundary, flagged, and the clipped page still fits the budget.
#[test]
fn an_oversized_description_is_clipped_on_a_char_boundary_and_flagged() -> TestResult {
    let project = activated_project("lib.rs", b"fn main() {}\n")?;
    let manager = sync::SourceSyncManager::default();
    // Five thousand characters, ten thousand UTF-8 bytes: a clip must
    // never split one of these two-byte characters.
    let description = "é".repeat(5_000);
    write_finding_with_description(&project, "lib.rs", "multibyte", &description, &manager)?;
    let _config = write_default_page_bytes(500)?;

    let response = finding_read(
        &Parameters(request(&project.path().to_string_lossy(), None)),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("read")?
    .0;

    ensure_eq(&response.findings.len(), &1, "response.findings.len() == 1")?;
    let entry = response.findings.first().check("response.findings[0]")?;
    ensure(entry.detail_clipped, "entry.detail_clipped")?;
    ensure(
        entry.description.chars().count() < description.chars().count(),
        "entry.description.chars().count() < description.chars().count()",
    )?;
    ensure(
        description.starts_with(&entry.description),
        "description.starts_with(&entry.description)",
    )?;
    ensure(
        response.page.bytes_used <= response.page.page_bytes,
        format!(
            "a clipped page must fit its budget: {} of {}",
            response.page.bytes_used, response.page.page_bytes
        ),
    )?;
    ensure(response.page.complete, "response.page.complete")?;
    Ok(())
}

#[test]
fn finding_read_rejects_an_oversized_cursor_with_invalid_params() -> TestResult {
    let error = finding_read(
        &Parameters(FindingReadRequest {
            path: None,
            file: None,
            cursor: Some("c".repeat(crate::tools::MAX_CURSOR_BYTES + 1)),
            page_bytes: None,
            legacy_paging: LegacyPaging::default(),
        }),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    )
    .err()
    .check("oversized cursor is refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &format!("cursor exceeds {} bytes", crate::tools::MAX_CURSOR_BYTES),
        "error.message names MAX_CURSOR_BYTES",
    )?;
    Ok(())
}

#[test]
fn finding_read_clips_to_empty_when_page_bytes_is_smaller_than_entry() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn f() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::default();
    let long_desc = "x".repeat(500);
    super::finding(
        &Parameters(super::FindingRequest {
            path: path.clone(),
            file: "lib.rs".to_owned(),
            line_start: 1,
            line_end: 1,
            severity: "medium".to_owned(),
            category: "correctness".to_owned(),
            title: "t".to_owned(),
            description: long_desc,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("insert finding")?;

    let mut req = request(&path, None);
    req.page_bytes = Some(32);
    let read = finding_read(
        &Parameters(req),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("finding_read under tiny page_bytes succeeds")?;
    let first = read.0.findings.first().check("one finding returned")?;
    ensure_eq(&first.description, &"", "description clipped to empty")?;
    ensure(first.detail_clipped, "detail_clipped must be true")?;
    Ok(())
}
