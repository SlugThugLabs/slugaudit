//! Tests for the `project_control` MCP tool: `on` creates `.planning/slugaudit/`
//! and runs the initial import, `off` clears session state and deletes the
//! runtime directory, and both handle idempotent and error states cleanly.

use super::*;
use crate::tools::MAX_PATH_BYTES;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

pub(super) fn temp_project_with_source() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join("src")).check("create src dir")?;
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

fn request(path: &str, action: ProjectControlAction) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action,
        profile: None,
    })
}

pub(super) fn on_request(path: &str) -> Parameters<ProjectControlRequest> {
    request(path, ProjectControlAction::On)
}

pub(super) fn off_request(path: &str) -> Parameters<ProjectControlRequest> {
    request(path, ProjectControlAction::Off)
}

pub(super) fn activation_dir(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning").join("slugaudit")
}

#[test]
fn on_and_off_actions_parse_and_path_defaults_to_none() -> TestResult {
    let on: ProjectControlRequest =
        serde_json::from_str(r#"{"action":"on"}"#).check("on deserializes")?;
    ensure(matches!(on.action, ProjectControlAction::On), "On")?;
    ensure(on.path.is_none(), "on.path.is_none()")?;

    let off: ProjectControlRequest =
        serde_json::from_str(r#"{"action":"off"}"#).check("off deserializes")?;
    ensure(matches!(off.action, ProjectControlAction::Off), "Off")?;
    Ok(())
}

#[test]
fn on_action_enables_the_project_and_runs_the_initial_import() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let response = project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("enable succeeds")?;

    let inner = response.0;
    ensure_eq(
        &inner.status,
        &ProjectControlStatus::Enabled,
        "inner.status == ProjectControlStatus::Enabled",
    )?;
    ensure_eq(
        &inner.path,
        &project
            .path()
            .canonicalize()
            .check("tempdir canonicalizes")?
            .to_string_lossy(),
        "the response reports the canonical project root",
    )?;
    let import = inner.import.check("import report present")?;
    ensure(
        import.added >= 1,
        "the initial import must index the source file",
    )?;
    ensure(
        !import.revision_id.is_empty(),
        "!import.revision_id.is_empty()",
    )?;
    ensure(
        activation_dir(&project).is_dir(),
        "the activation marker must exist after enable",
    )?;
    ensure(
        activation_dir(&project).join("project.db").exists(),
        "the project database must exist after enable",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn on_action_reports_files_the_initial_import_skipped() -> TestResult {
    use std::os::unix::ffi::OsStrExt;
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let unrepresentable = std::ffi::OsStr::from_bytes(b"bad\xffname.rs");
    fs::write(project.path().join(unrepresentable), b"pub fn b() {}\n")
        .check("write non-UTF-8 file")?;

    let response = project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("enable succeeds")?;

    let import = response.0.import.check("import report present")?;
    ensure_eq(
        &import.skipped,
        &1,
        "the skipped file must be reported in the response, not dropped",
    )?;
    ensure(
        import.added >= 1,
        "the representable source is still imported",
    )?;
    Ok(())
}

#[test]
fn on_action_is_idempotent_when_already_enabled() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("first enable")?;
    let response = project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("second enable")?;
    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::Enabled,
        "response.0.status == ProjectControlStatus::Enabled",
    )?;
    ensure(
        activation_dir(&project).is_dir(),
        "the directory stays in place across repeated enables",
    )?;
    Ok(())
}

#[test]
fn off_action_disables_the_project_and_deletes_the_directory() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("enable")?;
    ensure(
        activation_dir(&project).is_dir(),
        "activation_dir(&project).is_dir()",
    )?;

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("disable succeeds")?;
    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::Disabled,
        "response.0.status == ProjectControlStatus::Disabled",
    )?;
    ensure(response.0.import.is_none(), "response.0.import.is_none()")?;
    ensure(
        !activation_dir(&project).exists(),
        "disable must delete the whole activation directory",
    )?;
    Ok(())
}

#[test]
fn off_action_when_already_disabled_is_a_noop() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("noop disable")?;
    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::AlreadyDisabled,
        "response.0.status == ProjectControlStatus::AlreadyDisabled",
    )?;
    ensure(response.0.import.is_none(), "response.0.import.is_none()")?;
    ensure(
        !activation_dir(&project).exists(),
        "!activation_dir(&project).exists()",
    )?;
    Ok(())
}

#[test]
fn on_action_with_a_nonexistent_path_returns_invalid_params() -> TestResult {
    let missing = "/definitely/not/a/real/path/for/slugaudit-tests";
    let manager = sync::SourceSyncManager::new();
    let error = project_control(
        &on_request(missing),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("enable on a nonexistent path must surface an error")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        format!("a bad caller path is not an internal server failure: {error:?}"),
    )?;
    ensure_eq(
        &error.message,
        &format!("project path `{missing}` does not exist; pass an existing directory"),
        "error message names missing directory",
    )?;
    Ok(())
}

#[test]
fn project_control_rejects_a_path_over_the_byte_cap() -> TestResult {
    let manager = sync::SourceSyncManager::new();
    let over = MAX_PATH_BYTES.checked_add(1).check("cap + 1")?;
    let error = project_control(
        &on_request(&"a".repeat(over)),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("an over-cap path is refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        format!("{error:?}"),
    )?;
    ensure_eq(
        &error.message,
        &"path exceeds 32768 bytes",
        "error.message == \"path exceeds 32768 bytes\"",
    )?;
    Ok(())
}

#[test]
fn project_control_rejects_a_profile_over_the_byte_cap() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let mut request = on_request(&path);
    request.0.profile = Some("p".repeat(MAX_PROFILE_BYTES.checked_add(1).check("cap+1")?));

    let error = project_control(&request, &crate::progress::NoopProgressSink, &manager)
        .err()
        .check("an over-cap profile is refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        format!("{error:?}"),
    )?;
    ensure_eq(
        &error.message,
        &"profile exceeds 64 bytes",
        "error.message == \"profile exceeds 64 bytes\"",
    )?;
    ensure(
        !activation_dir(&project).exists(),
        "the refused call must not enable the project",
    )?;
    Ok(())
}

#[test]
fn on_action_reports_a_non_corruption_database_open_failure() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let database = activation_dir(&project).join("project.db");
    fs::create_dir_all(&database).check("a directory where the database belongs")?;

    let error = project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("an unopenable database fails the enable")?;

    ensure(
        error.message.starts_with("database:"),
        format!(
            "the failure is reported as a database error: {}",
            error.message
        ),
    )?;
    ensure(
        !error.message.contains("discarding"),
        format!("an I/O failure is not corruption: {}", error.message),
    )?;
    Ok(())
}

#[test]
fn on_action_reports_an_import_failure() -> TestResult {
    let (_project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let _config =
        crate::tools::test_support::use_global_config(r#"{"max_total_import_bytes": 1}"#)?;

    let error = project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("an import that exceeds the byte ceiling fails the enable")?;

    ensure(
        error.message.contains("import:"),
        format!("the failure names the import step: {}", error.message),
    )?;
    ensure(
        error.message.contains("1-byte ceiling"),
        format!("the underlying import error survives: {}", error.message),
    )?;
    Ok(())
}

#[test]
fn off_action_reports_a_disable_failure() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::new();
    let planning = project.path().join(".planning");
    fs::write(&planning, b"not a directory").check("plant a file as .planning")?;

    let error = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .map(|_| ())
    .check_err("disable must fail when .planning is not a directory")?;

    ensure(
        error.message.contains("disable"),
        format!("the error must name the disable failure: {}", error.message),
    )?;
    ensure(
        planning.is_file(),
        "a failed disable must leave .planning untouched",
    )?;
    Ok(())
}

fn mechanic_action(
    manager: &sync::SourceSyncManager,
    path: &str,
    action: crate::tools::MechanicAction,
    add_spec: Option<(&str, &str, &str, &str)>,
) -> TestResult<crate::tools::MechanicResponse> {
    let (id, language, query, note) = match add_spec {
        Some((id, language, query, note)) => (
            Some(id.to_owned()),
            Some(language.to_owned()),
            Some(query.to_owned()),
            Some(note.to_owned()),
        ),
        None => (None, None, None, None),
    };
    crate::tools::mechanic(
        &Parameters(crate::tools::MechanicRequest {
            path: Some(path.to_owned()),
            action: Some(action),
            cursor: None,
            legacy_paging: crate::tools::LegacyPaging::default(),
            id,
            language,
            query,
            note,
            limit: None,
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("mechanic call succeeds")
    .map(|rmcp::handler::server::wrapper::Json(resp)| resp)
}

#[test]
fn on_and_off_stamp_sync_clear_mechanics_and_release_result_stores() -> TestResult {
    let (_project, path) = temp_project_with_source()?;
    let manager = sync::SourceSyncManager::default();
    ensure_eq(
        &manager.last_sync_unix_seconds(),
        &0,
        "no sync has run before enable",
    )?;

    project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("enable with manager succeeds")?;
    ensure(
        manager.last_sync_unix_seconds() > 0,
        "enable through SourceSyncManager stamps last_sync_unix_seconds",
    )?;

    mechanic_action(
        &manager,
        &path,
        crate::tools::MechanicAction::Add,
        Some((
            "fn_items",
            "rust",
            "(function_item name: (identifier) @n)",
            "functions",
        )),
    )?;

    let page1 = crate::tools::query(
        &Parameters(crate::tools::QueryRequest {
            path: Some(path.clone()),
            sql: "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<40) SELECT i, printf('%040d', i) AS pad FROM n".to_owned(),
            cursor: None,
            legacy_paging: crate::tools::LegacyPaging::default(),
            page_bytes: Some(400),
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("multi-page query succeeds")?
    .0;
    let cursor = page1
        .page
        .cursor
        .check("multi-page query leaves a cursor")?;

    project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("disable with manager succeeds")?;

    ensure(
        manager
            .result_stores()
            .current()
            .check("store slot")?
            .is_none(),
        "disable releases the project's result store",
    )?;
    let stale_err = crate::tools::query(
        &Parameters(crate::tools::QueryRequest {
            path: None,
            sql: String::new(),
            cursor: Some(cursor),
            legacy_paging: crate::tools::LegacyPaging::default(),
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .err()
    .check("pre-off cursor must fail after disable")?;
    ensure_eq(
        &stale_err
            .data
            .as_ref()
            .and_then(|d| d.get("code"))
            .and_then(serde_json::Value::as_str),
        &Some("unknown_cursor"),
        "released result store invalidates pre-off cursors",
    )?;

    project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("re-enable succeeds")?;
    let listed = mechanic_action(&manager, &path, crate::tools::MechanicAction::List, None)?;
    ensure(
        listed.mechanics.iter().all(|m| m.id != "fn_items"),
        "project_control off cleared custom mechanics for that project",
    )?;
    Ok(())
}
