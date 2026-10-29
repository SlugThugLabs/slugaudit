use super::*;
use std::path::PathBuf;

#[test]
fn a_config_write_io_failure_is_an_internal_error_naming_the_cause() {
    let error = project::ConfigWriteError::Io {
        path: PathBuf::from("/project/.planning/slugaudit/config.json"),
        source: std::io::Error::other("no space left on device"),
    };

    let mapped = config_write_error(error);

    assert_eq!(
        mapped.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{mapped:?}"
    );
    assert!(
        mapped.message.contains("write profile config")
            && mapped.message.contains("no space left on device"),
        "the message keeps the operation and the source error: {}",
        mapped.message
    );
}

#[test]
fn a_non_regular_config_target_is_invalid_params() {
    let error = project::ConfigWriteError::NotRegularFile {
        path: PathBuf::from("/project/.planning/slugaudit/config.json"),
    };

    let mapped = config_write_error(error);

    assert_eq!(
        mapped.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{mapped:?}"
    );
    assert!(mapped.message.contains("config.json"), "{}", mapped.message);
}

#[test]
fn a_config_locked_by_another_process_is_an_internal_error() {
    let error = project::ConfigWriteError::Locked {
        path: PathBuf::from("/project/.planning/slugaudit/config.lock"),
        waited_ms: 5000,
    };

    let mapped = config_write_error(error);

    assert_eq!(
        mapped.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{mapped:?}"
    );
    assert!(
        mapped.message.contains("locked by another process"),
        "{}",
        mapped.message
    );
}

/// A project with one source file and a config holding a limit and a
/// persisted mechanic, as `mechanic add persist: true` leaves it.
fn project_with_persisted_mechanic() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    std::fs::write(project.path().join("app.py"), b"eval('1')\n").expect("write source");
    let dir = project.path().join(".planning/slugaudit");
    std::fs::create_dir_all(&dir).expect("activation dir");
    let config = serde_json::json!({
        "default_page_bytes": 4096,
        "mechanics": {"version": 1, "entries": [{
            "id": "py_exec", "language": "python",
            "query": "(call function: (identifier) @callee (#eq? @callee \"exec\"))",
            "note": "dynamic code execution"
        }]}
    })
    .to_string();
    std::fs::write(dir.join("config.json"), config).expect("write config");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

fn on_with_profile(path: &str, profile: &str) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action: ProjectControlAction::On,
        profile: Some(profile.to_owned()),
    })
}

#[test]
fn turning_on_with_a_profile_keeps_persisted_mechanics_and_every_other_key() {
    let (project, path) = project_with_persisted_mechanic();

    project_control(
        &on_with_profile(&path, "lean"),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("enable with a profile");

    let written: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(project.path().join(".planning/slugaudit/config.json"))
            .expect("read config"),
    )
    .expect("config stays JSON");
    assert_eq!(written["profile"], "lean");
    assert_eq!(written["default_page_bytes"], 4096);
    assert_eq!(written["mechanics"]["entries"][0]["id"], "py_exec");
    assert_eq!(project::load_persisted(project.path()).entries.len(), 1);
}

#[test]
fn turning_on_with_a_profile_refuses_a_malformed_config_and_leaves_it_untouched() {
    let (project, path) = project_with_persisted_mechanic();
    let config_path = project.path().join(".planning/slugaudit/config.json");
    std::fs::write(&config_path, "{\"mechanics\": ").expect("truncate config");

    let error = project_control(
        &on_with_profile(&path, "lean"),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .expect("a config it cannot parse is never overwritten");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error.message.contains("not valid JSON"),
        "{}",
        error.message
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("read config"),
        "{\"mechanics\": "
    );
}

fn control(path: &str, action: ProjectControlAction) -> Json<ProjectControlResponse> {
    project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path.to_owned()),
            action,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("project_control succeeds")
}

fn indexed_project() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

#[test]
fn a_tool_call_after_project_off_is_not_active_and_does_not_rebuild_the_database() {
    let (project, path) = indexed_project();
    control(&path, ProjectControlAction::On);
    let activation = project.path().join(".planning/slugaudit");
    std::fs::write(activation.join("config.json"), b"{}").expect("write config");

    let off = control(&path, ProjectControlAction::Off).0;

    assert_eq!(off.status, "disabled");
    assert!(off.config_kept);
    assert!(!activation.join("project.db").exists());
    assert!(!activation.join("results").exists());
    let error = crate::tools::report(
        &Parameters(crate::tools::ReportRequest { path: path.clone() }),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::new(),
    )
    .err()
    .expect("a call on a project that is off fails");
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!("project_not_enabled")),
        "{error:?}"
    );
    assert!(
        !activation.join("project.db").exists(),
        "the failed call must not rebuild the database"
    );
    assert_eq!(
        control(&path, ProjectControlAction::Off).0.status,
        "already_disabled",
        "a second off has nothing left to remove"
    );
}

#[test]
fn project_off_then_on_restores_the_mechanics_persisted_in_config_json() {
    let (project, path) = indexed_project();
    control(&path, ProjectControlAction::On);
    let root = ProjectRoot::resolve(project.path()).expect("resolve root");
    let entry = project::MechanicEntry {
        id: "py_eval".to_owned(),
        language: "python".to_owned(),
        query: "(call function: (identifier) @c)".to_owned(),
        note: "counts calls".to_owned(),
    };
    project::persist_entry(root.as_path(), &entry).expect("persist the mechanic");

    control(&path, ProjectControlAction::Off);
    let on = control(&path, ProjectControlAction::On).0;

    assert_eq!(on.status, "enabled");
    let restored = project::load_persisted(root.as_path());
    assert!(restored.invalid.is_empty(), "{:?}", restored.invalid);
    assert_eq!(
        restored
            .entries
            .iter()
            .map(|(_, restored)| restored)
            .collect::<Vec<_>>(),
        vec![&entry]
    );
}

#[test]
fn off_fails_when_the_activation_dir_cannot_be_inspected() {
    let (project, path) = indexed_project();
    // `.planning` is a file, so `.planning/slugaudit` fails with ENOTDIR.
    std::fs::write(project.path().join(".planning"), b"x").expect("write file");

    let error = project_control(
        &Parameters(ProjectControlRequest {
            path: Some(path),
            action: ProjectControlAction::Off,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .expect("an uninspectable activation dir is not 'already disabled'");

    assert_eq!(
        error.message,
        "disable: failed to update the activation directory: Not a directory (os error 20)",
        "an uninspectable activation dir fails the operation with its cause"
    );
}
