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
