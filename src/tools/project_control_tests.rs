//! Tests for the `project_control` MCP tool: `on` creates the activation
//! marker and runs the initial import, `off` purges the marker and the
//! database, and both degrade gracefully when the project is already in
//! the requested state.

use super::*;
use std::fs;

fn temp_project_with_source() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join("src")).expect("create src dir");
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
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
fn on_action_parses() {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).expect("request deserializes");
    assert!(matches!(req.action, ProjectControlAction::On));
}

#[test]
fn off_action_parses() {
    let json = r#"{"action":"off"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).expect("request deserializes");
    assert!(matches!(req.action, ProjectControlAction::Off));
}

#[test]
fn path_defaults_to_none() {
    let json = r#"{"action":"on"}"#;
    let req: ProjectControlRequest = serde_json::from_str(json).expect("request deserializes");
    assert!(req.path.is_none());
}

#[test]
fn on_action_enables_the_project_and_runs_the_initial_import() {
    let (project, path) = temp_project_with_source();
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .expect("enable succeeds");

    let inner = response.0;
    assert_eq!(inner.status, "enabled");
    assert_eq!(
        inner.path,
        project
            .path()
            .canonicalize()
            .expect("tempdir canonicalizes")
            .to_string_lossy(),
        "the response reports the canonical project root"
    );
    let import = inner.import.expect("import report present");
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
}

#[test]
fn on_action_is_idempotent_when_already_enabled() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .expect("first enable");
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .expect("second enable");
    assert_eq!(response.0.status, "enabled");
    assert!(
        activation_dir(&project).is_dir(),
        "the marker stays in place across repeated enables"
    );
}

#[test]
fn off_action_disables_the_project_and_purges_the_database() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None).expect("enable");
    assert!(activation_dir(&project).is_dir());

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("disable succeeds");
    assert_eq!(response.0.status, "disabled");
    assert!(response.0.import.is_none());
    assert!(
        !activation_dir(&project).exists(),
        "disable must purge the activation marker and the database together"
    );
}

#[test]
fn off_action_unwatches_project_in_sync_manager() {
    let (project, path) = temp_project_with_source();
    let manager = sync::SourceSyncManager::new();
    manager.activate(project.path());
    assert!(manager.active_watch_state().is_some());

    project_control(
        &on_request(&path),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("enable");
    project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("disable");

    assert!(
        manager.active_watch_state().is_none(),
        "disabling project must unregister watcher"
    );
}

#[test]
fn off_action_when_already_disabled_is_a_noop() {
    let (project, path) = temp_project_with_source();
    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("noop disable");
    assert_eq!(response.0.status, "already_disabled");
    assert!(response.0.import.is_none());
    assert!(!activation_dir(&project).exists());
}

#[test]
fn on_action_with_a_nonexistent_path_returns_an_error() {
    let missing = "/definitely/not/a/real/path/for/slugaudit-tests";
    let result = project_control(
        &on_request(missing),
        &crate::progress::NoopProgressSink,
        None,
    );
    assert!(
        result.is_err(),
        "enable on a nonexistent path must surface an error, not silently create state"
    );
}

#[test]
fn on_action_with_profile_writes_config_json() {
    let (project, path) = temp_project_with_source();
    let req = Parameters(ProjectControlRequest {
        path: Some(path),
        action: ProjectControlAction::On,
        profile: Some("audit".to_string()),
    });
    let response = project_control(&req, &crate::progress::NoopProgressSink, None)
        .expect("enable with profile succeeds");
    assert_eq!(response.0.status, "enabled");

    let cfg_file = activation_dir(&project).join("config.json");
    assert!(cfg_file.exists(), "config.json should be written");
    let content = fs::read_to_string(cfg_file).expect("read config");
    assert!(content.contains("\"audit\""));
}

#[test]
fn on_action_with_profile_reports_config_write_failure() {
    let (project, path) = temp_project_with_source();
    let config_path = activation_dir(&project).join("config.json");
    fs::create_dir_all(&config_path).expect("create config parent");
    let req = Parameters(ProjectControlRequest {
        path: Some(path),
        action: ProjectControlAction::On,
        profile: Some("audit".to_string()),
    });

    let result = project_control(&req, &crate::progress::NoopProgressSink, None);
    assert!(
        result.is_err(),
        "a failed profile write must fail the enable request"
    );
    if let Err(error) = result {
        assert!(
            error.message.contains("write profile config"),
            "error must identify the failed config write: {}",
            error.message
        );
    }
}

#[test]
fn on_action_reports_the_gitignore_rule_it_added() {
    let (project, path) = temp_project_with_source();
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .expect("enable succeeds");

    let report = response
        .0
        .gitignore
        .expect("enable reports the gitignore guard");
    let expected = project
        .path()
        .canonicalize()
        .expect("tempdir canonicalizes")
        .join(".gitignore");
    assert_eq!(report.status, GitignoreStatus::Added);
    assert_eq!(report.path, expected.to_string_lossy());
    assert!(
        report.error.is_none(),
        "a successful append carries no error"
    );
    assert!(
        fs::read_to_string(&expected)
            .expect("read .gitignore")
            .contains(".planning/slugaudit/"),
        "the rule is on disk"
    );
}

#[test]
fn on_action_reports_an_already_ignored_project_without_duplicating_the_rule() {
    let (project, path) = temp_project_with_source();
    let gitignore = project.path().join(".gitignore");
    let original = b"/target\n/.planning/slugaudit/\n";
    fs::write(&gitignore, original).expect("write .gitignore");

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .expect("enable succeeds");

    let report = response
        .0
        .gitignore
        .expect("enable reports the gitignore guard");
    assert_eq!(report.status, GitignoreStatus::AlreadyIgnored);
    assert_eq!(response.0.status, "enabled");
    assert!(response.0.import.is_some(), "the import still ran");
    assert_eq!(
        fs::read(&gitignore).expect("read .gitignore"),
        original.to_vec(),
        "an already-ignored project's file stays byte-identical"
    );
}

#[test]
fn on_action_enables_the_project_even_when_the_gitignore_cannot_be_written() {
    let (project, path) = temp_project_with_source();
    fs::create_dir(project.path().join(".gitignore")).expect("block the ignore file");

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .expect("the convenience guard must not fail the enable");

    let report = response
        .0
        .gitignore
        .expect("enable reports the gitignore guard");
    assert_eq!(report.status, GitignoreStatus::NotWritten);
    assert!(report.error.is_some(), "the reason is reported");
    assert_eq!(response.0.status, "enabled");
    assert!(
        activation_dir(&project).join("project.db").exists(),
        "the project is enabled regardless of the ignore guard"
    );
}

#[test]
fn off_action_does_not_touch_the_gitignore() {
    let (project, path) = temp_project_with_source();
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None).expect("enable");
    let gitignore = project.path().join(".gitignore");
    let after_enable = fs::read(&gitignore).expect("read .gitignore");

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("disable succeeds");

    assert_eq!(response.0.status, "disabled");
    assert!(
        response.0.gitignore.is_none(),
        "off omits the gitignore field"
    );
    assert_eq!(
        fs::read(&gitignore).expect("read .gitignore"),
        after_enable,
        "off leaves the user's ignore file alone"
    );
}

#[test]
fn an_unknown_profile_is_refused_before_anything_is_enabled() {
    let (project, path) = temp_project_with_source();
    let req = Parameters(ProjectControlRequest {
        path: Some(path),
        action: ProjectControlAction::On,
        profile: Some("strict".to_string()),
    });

    let error = project_control(&req, &crate::progress::NoopProgressSink, None)
        .err()
        .expect("a profile that does not exist must not enable on the default");

    assert!(
        error.message.contains("unknown profile `strict`"),
        "{}",
        error.message
    );
    assert!(
        !activation_dir(&project).exists(),
        "the refused call must leave the project disabled"
    );
}

#[test]
fn off_fails_when_the_activation_dir_cannot_be_inspected() {
    let (project, path) = temp_project_with_source();
    // `.planning` is a file, so `.planning/slugaudit` fails with ENOTDIR.
    fs::write(project.path().join(".planning"), b"x").expect("write file");

    let error = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .expect("an uninspectable activation dir is not 'already disabled'");

    assert!(error.message.contains("inspect"), "{}", error.message);
}
