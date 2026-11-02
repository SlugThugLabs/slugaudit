//! Tests for the `project_control` MCP tool: `on` creates the activation
//! marker and runs the initial import, `off` purges the marker and the
//! database, and both degrade gracefully when the project is already in
//! the requested state.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};
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
    assert!(matches!(req.action, ProjectControlAction::On));
    Ok(())
}

#[test]
fn off_action_parses() -> TestResult {
    let json = r#"{"action":"off"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).check("request deserializes")?;
    assert!(matches!(req.action, ProjectControlAction::Off));
    Ok(())
}

#[test]
fn path_defaults_to_none() -> TestResult {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).check("request deserializes")?;
    assert!(req.path.is_none());
    Ok(())
}

#[test]
fn on_action_enables_the_project_and_runs_the_initial_import() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable succeeds")?;

    let inner = response.0;
    assert_eq!(inner.status, "enabled");
    assert_eq!(
        inner.path,
        project
            .path()
            .canonicalize()
            .check("tempdir canonicalizes")?
            .to_string_lossy(),
        "the response reports the canonical project root"
    );
    let import = inner.import.check("import report present")?;
    assert!(
        import.added >= 1,
        "the initial import must index the source file"
    );
    assert!(!import.revision_id.is_empty());

    assert!(
        activation_dir(&project).is_dir(),
        "the activation marker must exist after enable"
    );
    assert!(
        activation_dir(&project).join("project.db").exists(),
        "the project database must exist after enable"
    );
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
    assert_eq!(
        import.skipped, 1,
        "the skipped file must be reported in the response, not dropped"
    );
    assert!(
        import.added >= 1,
        "the representable source is still imported"
    );
    Ok(())
}

#[test]
fn on_action_is_idempotent_when_already_enabled() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("first enable")?;
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("second enable")?;
    assert_eq!(response.0.status, "enabled");
    assert!(
        activation_dir(&project).is_dir(),
        "the directory stays in place across repeated enables"
    );
    Ok(())
}

#[test]
fn off_action_disables_the_project_and_deletes_the_directory() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable")?;
    assert!(activation_dir(&project).is_dir());

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .check("disable succeeds")?;
    assert_eq!(response.0.status, "disabled");
    assert!(response.0.import.is_none());
    assert!(
        !activation_dir(&project).exists(),
        "disable must delete the whole activation directory"
    );
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
    assert_eq!(response.0.status, "already_disabled");
    assert!(response.0.import.is_none());
    assert!(!activation_dir(&project).exists());
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

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "a bad caller path is not an internal server failure: {error:?}"
    );
    assert_eq!(
        error.message,
        format!("project path `{missing}` does not exist; pass an existing directory")
    );
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

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert_eq!(error.message, "path exceeds 32768 bytes");
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

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert_eq!(error.message, "profile exceeds 64 bytes");
    assert!(
        !activation_dir(&project).exists(),
        "the refused call must not enable the project"
    );
    Ok(())
}

/// `project::enable` refuses to create the activation directory through a
/// symlinked `.planning`, and that refusal reaches the caller as itself.
#[cfg(unix)]
#[test]
fn on_action_reports_a_symlinked_planning_directory() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let elsewhere = project.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).check("create link target")?;
    std::os::unix::fs::symlink(&elsewhere, project.path().join(".planning")).check("symlink")?;

    let error = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .err()
        .check("enable through a symlinked .planning is refused")?;

    assert!(
        error
            .message
            .contains("enable: refusing a symlinked SlugAudit activation path"),
        "{}",
        error.message
    );
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
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(
        error.message,
        format!("discarding outdated database: {discard_error}")
    );
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

    let open_error = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&database)
        .check_err("the parent directory is missing")?;
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(
        error.message,
        format!("database: failed to create the database file: {open_error}")
    );
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

    assert!(
        error.message.starts_with("database:"),
        "the failure is reported as a database error: {}",
        error.message
    );
    assert!(
        !error.message.contains("discarding"),
        "an I/O failure is not corruption: {}",
        error.message
    );
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

    assert!(
        error.message.contains("import:"),
        "the failure names the import step: {}",
        error.message
    );
    assert!(
        error.message.contains("1-byte ceiling"),
        "the underlying import error survives: {}",
        error.message
    );
    Ok(())
}

/// `project::disable` refuses to remove through a symlinked `.planning`;
/// that refusal reaches the caller instead of a reported success.
#[cfg(unix)]
#[test]
fn off_action_reports_a_disable_failure() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let elsewhere = project.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join("slugaudit")).check("create link target")?;
    std::os::unix::fs::symlink(&elsewhere, project.path().join(".planning"))
        .check("symlink .planning")?;

    let error = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .check("disable through a symlinked .planning is refused")?;

    assert!(
        error
            .message
            .contains("disable: refusing a symlinked SlugAudit activation path"),
        "{}",
        error.message
    );
    assert!(elsewhere.join("slugaudit").is_dir(), "nothing was removed");
    Ok(())
}

#[test]
fn on_action_reports_the_gitignore_rule_it_added() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable succeeds")?;

    let report = response
        .0
        .gitignore
        .check("enable reports the gitignore guard")?;
    let expected = project
        .path()
        .canonicalize()
        .check("tempdir canonicalizes")?
        .join(".gitignore");
    assert_eq!(report.status, GitignoreStatus::Added);
    assert_eq!(report.path, expected.to_string_lossy());
    assert!(
        report.error.is_none(),
        "a successful append carries no error"
    );
    assert!(
        fs::read_to_string(&expected)
            .check("read .gitignore")?
            .contains(".planning/slugaudit/"),
        "the rule is on disk"
    );
    Ok(())
}

#[test]
fn on_action_reports_an_already_ignored_project_without_duplicating_the_rule() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let gitignore = project.path().join(".gitignore");
    let original = b"/target\n/.planning/slugaudit/\n";
    fs::write(&gitignore, original).check("write .gitignore")?;

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable succeeds")?;

    let report = response
        .0
        .gitignore
        .check("enable reports the gitignore guard")?;
    assert_eq!(report.status, GitignoreStatus::AlreadyIgnored);
    assert_eq!(response.0.status, "enabled");
    assert!(response.0.import.is_some(), "the import still ran");
    assert_eq!(
        fs::read(&gitignore).check("read .gitignore")?,
        original.to_vec(),
        "an already-ignored project's file stays byte-identical"
    );
    Ok(())
}

#[test]
fn on_action_enables_the_project_even_when_the_gitignore_cannot_be_written() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    fs::create_dir(project.path().join(".gitignore")).check("block the ignore file")?;

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("the convenience guard must not fail the enable")?;

    let report = response
        .0
        .gitignore
        .check("enable reports the gitignore guard")?;
    assert_eq!(report.status, GitignoreStatus::NotWritten);
    assert!(report.error.is_some(), "the reason is reported");
    assert_eq!(response.0.status, "enabled");
    assert!(
        activation_dir(&project).join("project.db").exists(),
        "the project is enabled regardless of the ignore guard"
    );
    Ok(())
}

#[test]
fn off_action_does_not_touch_the_gitignore() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable")?;
    let gitignore = project.path().join(".gitignore");
    let after_enable = fs::read(&gitignore).check("read .gitignore")?;

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .check("disable succeeds")?;

    assert_eq!(response.0.status, "disabled");
    assert!(
        response.0.gitignore.is_none(),
        "off omits the gitignore field"
    );
    assert_eq!(
        fs::read(&gitignore).check("read .gitignore")?,
        after_enable,
        "off leaves the user's ignore file alone"
    );
    Ok(())
}

#[test]
fn an_unknown_profile_is_refused_before_anything_is_enabled() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let req = Parameters(ProjectControlRequest {
        path: Some(path),
        action: ProjectControlAction::On,
        profile: Some("strict".to_string()),
    });

    let error = project_control(&req, &crate::progress::NoopProgressSink, None)
        .err()
        .check("a profile that does not exist must not enable on the default")?;

    assert!(
        error.message.contains("unknown profile `strict`"),
        "{}",
        error.message
    );
    assert!(
        !activation_dir(&project).exists(),
        "the refused call must leave the project disabled"
    );
    Ok(())
}
