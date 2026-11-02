use super::*;
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;
use crate::tools::{FindingReadRequest, FindingRequest, LegacyPaging, finding, finding_read};
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult};

fn write(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    line: u32,
    title: &str,
) -> TestResult<i64> {
    Ok(finding(
        &Parameters(FindingRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: "lib.rs".to_owned(),
            line_start: line,
            line_end: line,
            severity: "low".to_owned(),
            category: "test".to_owned(),
            title: title.to_owned(),
            description: "d".to_owned(),
        }),
        &NoopProgressSink,
        manager,
    )
    .check("finding succeeds")?
    .0
    .id)
}

fn read_titles(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
) -> TestResult<Vec<String>> {
    Ok(finding_read(
        &Parameters(FindingReadRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            file: None,
            cursor: None,
            legacy_paging: LegacyPaging::default(),
        }),
        &NoopProgressSink,
        manager,
    )
    .check("finding_read succeeds")?
    .0
    .findings
    .into_iter()
    .map(|entry| entry.title)
    .collect())
}

fn clear(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
) -> Result<FindingClearResponse, ErrorData> {
    finding_clear(
        &Parameters(FindingClearRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &NoopProgressSink,
        manager,
    )
    .map(|response| response.0)
}

#[test]
fn clear_deletes_every_finding_and_reports_how_many() -> TestResult {
    let project = activated_project("lib.rs", b"line one\nline two\nline three\n")?;
    let manager = SourceSyncManager::default();
    for (line, title) in [(1, "first"), (2, "second"), (3, "third")] {
        write(&project, &manager, line, title)?;
    }
    assert_eq!(read_titles(&project, &manager)?.len(), 3);

    let response = clear(&project, &manager).check("clear succeeds")?;

    assert_eq!(response.removed, 3);
    assert!(read_titles(&project, &manager)?.is_empty());
    assert_eq!(clear(&project, &manager).check("second clear")?.removed, 0);
    Ok(())
}

#[test]
fn clear_without_any_finding_reports_zero_and_creates_no_database() -> TestResult {
    let project = activated_project("lib.rs", b"line one\n")?;
    let manager = SourceSyncManager::default();

    let response = clear(&project, &manager).check("clear succeeds")?;

    assert_eq!(response.removed, 0);
    assert!(!crate::tools::test_support::own_findings_path(&project)?.exists());
    Ok(())
}

#[test]
fn findings_written_after_a_clear_start_fresh() -> TestResult {
    let project = activated_project("lib.rs", b"line one\nline two\n")?;
    let manager = SourceSyncManager::default();
    write(&project, &manager, 1, "before")?;
    clear(&project, &manager).check("clear succeeds")?;

    write(&project, &manager, 2, "after")?;

    assert_eq!(read_titles(&project, &manager)?, ["after"]);
    Ok(())
}

#[test]
fn clear_only_touches_the_named_project() -> TestResult {
    let first = activated_project("lib.rs", b"line one\n")?;
    let second = activated_project("lib.rs", b"line one\n")?;
    let manager = SourceSyncManager::default();
    write(&first, &manager, 1, "first project")?;
    write(&second, &manager, 1, "second project")?;

    clear(&first, &manager).check("clear succeeds")?;

    assert!(read_titles(&first, &manager)?.is_empty());
    assert_eq!(read_titles(&second, &manager)?, ["second project"]);
    Ok(())
}

#[test]
fn clear_refuses_an_oversized_path_and_a_project_that_is_not_enabled() -> TestResult {
    let manager = SourceSyncManager::default();
    let Err(oversized) = finding_clear(
        &Parameters(FindingClearRequest {
            path: "x".repeat(MAX_PATH_BYTES + 1),
        }),
        &NoopProgressSink,
        &manager,
    ) else {
        return Err("an oversized path is refused".to_owned()).check("unexpected outcome");
    };
    assert_eq!(oversized.message, "path exceeds its input size limit");
    assert_eq!(oversized.code, rmcp::model::ErrorCode::INVALID_PARAMS);

    let bare = tempfile::tempdir().check("bare dir")?;
    let error = clear(&bare, &manager).check_err("no project here")?;
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!("project_not_enabled")),
        "{error:?}"
    );
    Ok(())
}
