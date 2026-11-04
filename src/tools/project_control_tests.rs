//! Tests for the `project_control` MCP tool: `on` creates the activation
//! marker and runs the initial import, `off` purges the marker and the
//! database, and both degrade gracefully when the project is already in
//! the requested state.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

fn temp_project_with_source() -> TestResult<(tempfile::TempDir, String)> {
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

fn on_request(path: &str) -> Parameters<ProjectControlRequest> {
    request(path, ProjectControlAction::On)
}

fn off_request(path: &str) -> Parameters<ProjectControlRequest> {
    request(path, ProjectControlAction::Off)
}

fn activation_dir(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning").join("slugaudit")
}

#[test]
fn on_action_parses() -> TestResult {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).check("request deserializes")?;
    ensure(
        matches!(req.action, ProjectControlAction::On),
        "matches!(req.action, ProjectControlAction::On)",
    )?;
    Ok(())
}

#[test]
fn off_action_parses() -> TestResult {
    let json = r#"{"action":"off"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).check("request deserializes")?;
    ensure(
        matches!(req.action, ProjectControlAction::Off),
        "matches!(req.action, ProjectControlAction::Off)",
    )?;
    Ok(())
}

#[test]
fn path_defaults_to_none() -> TestResult {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).check("request deserializes")?;
    ensure(req.path.is_none(), "req.path.is_none()")?;
    Ok(())
}

#[test]
fn on_action_enables_the_project_and_runs_the_initial_import() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
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

/// A file discovery cannot include is a producer gap; the enable response
/// must carry it rather than report an import as if every discovered file
/// had been read. A non-UTF-8 name is the hermetic fixture: it can never
/// get a `files` row, so the publish report must list it in `skipped`.
#[cfg(unix)]
#[test]
fn on_action_reports_files_the_initial_import_skipped() -> TestResult {
    use std::os::unix::ffi::OsStrExt;
    let (project, path) = temp_project_with_source()?;
    let unrepresentable = std::ffi::OsStr::from_bytes(b"bad\xffname.rs");
    fs::write(project.path().join(unrepresentable), b"pub fn b() {}\n")
        .check("write non-UTF-8 file")?;

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
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
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("first enable")?;
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
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
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable")?;
    ensure(
        activation_dir(&project).is_dir(),
        "activation_dir(&project).is_dir()",
    )?;

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
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
    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
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
    let error = project_control(
        &on_request(missing),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .check("enable on a nonexistent path must surface an error, not silently create state")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        format!("a bad caller path is not an internal server failure: {error:?}"),
    )?;
    ensure_eq(
        &error.message,
        &format!("project path `{missing}` does not exist; pass an existing directory"),
        "error.message == format!(\"project path `{missing}` does not exist; pass an existing directory\")",
    )?;
    Ok(())
}

#[test]
fn project_control_rejects_a_path_over_the_byte_cap() -> TestResult {
    let error = project_control(
        &on_request(&"a".repeat(MAX_PATH_BYTES + 1)),
        &crate::progress::NoopProgressSink,
        None,
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
    let mut request = on_request(&path);
    request.0.profile = Some("p".repeat(MAX_PROFILE_BYTES + 1));

    let error = project_control(&request, &crate::progress::NoopProgressSink, None)
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

/// The recovery path discards the corrupt database before reopening it. A
/// directory at the database path cannot be unlinked, so the discard failure
/// must reach the caller with the underlying cause intact.
#[test]
fn corrupt_database_recovery_reports_a_failed_discard() -> TestResult {
    let directory = tempfile::tempdir().check("db dir")?;
    let database = directory.path().join("project.db");
    fs::create_dir(&database).check("directory at the database path")?;

    let error = recover_corrupt_database(&database)
        .check_err("a directory at the database path cannot be discarded")?;

    let discard_error = fs::remove_file(&database)
        .check_err("the directory at the database path must not unlink")?;
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure_eq(
        &error.message,
        &format!("discarding outdated database: {discard_error}"),
        "error.message == format!(\"discarding outdated database: {discard_error}\")",
    )?;
    Ok(())
}

/// After a successful discard the reopen must still fail closed with the
/// database context. A database under a missing parent directory reaches the
/// reopen arm on every platform, including when the tests run as root.
#[cfg(unix)]
#[test]
fn corrupt_database_recovery_reports_a_failed_reopen() -> TestResult {
    let directory = tempfile::tempdir().check("db dir")?;
    let database = directory.path().join("missing-parent").join("project.db");

    let error = recover_corrupt_database(&database)
        .check_err("a database under a missing parent cannot be recreated")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure_eq(
        &error.message,
        &format!(
            "database: failed to open database: unable to open database file: {}",
            database.display()
        ),
        "the reopen failure names the database path",
    )?;
    Ok(())
}

/// A database that cannot be opened for a reason other than corruption
/// (here the path is a directory) is reported as a database failure, not
/// misread as corruption to discard.
#[test]
fn on_action_reports_a_non_corruption_database_open_failure() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let activation = activation_dir(&project);
    let database = activation.join("project.db");
    fs::create_dir_all(&database).check("a directory where the database belongs")?;

    let error = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
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

/// The initial import runs inside `project_control`; its failure is
/// reported as an import failure rather than as success.
#[test]
fn on_action_reports_an_import_failure() -> TestResult {
    let (_project, path) = temp_project_with_source()?;
    let _config =
        crate::tools::test_support::use_global_config(r#"{"max_total_import_bytes": 1}"#)?;

    let error = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
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

#[path = "project_control_gitignore_tests.rs"]
mod gitignore_tests;

#[test]
fn off_action_reports_a_disable_failure() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let planning = project.path().join(".planning");
    fs::write(&planning, b"not a directory").check("plant a file as .planning")?;

    let error = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
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
