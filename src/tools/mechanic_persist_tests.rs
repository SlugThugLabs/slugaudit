//! `mechanic` with `persist: true`: the entry outlives the server in
//! `config.json`, every other key survives the write, a symlinked config is
//! refused, and a broken persisted entry is reported rather than dropped.
use super::tests::{
    EVAL_QUERY, action, add, call, leads_request, path_of, python_project, report_of, run_leads,
    sources,
};
use super::*;
use crate::tools::LeadMode;
use std::fs;

fn config_path(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning/slugaudit/config.json")
}

fn persisted(path: &str, id: &str) -> MechanicRequest {
    let mut request = add(path, id, "python", EVAL_QUERY);
    request.persist = true;
    request
}

#[test]
fn a_persisted_mechanic_is_reloaded_by_a_new_server_and_a_session_one_is_not() {
    let project = python_project();
    let path = path_of(&project);
    let first_server = sync::SourceSyncManager::default();
    call(&first_server, persisted(&path, "py_eval_saved")).expect("persist");
    call(
        &first_server,
        add(&path, "py_eval_session", "python", EVAL_QUERY),
    )
    .expect("add");

    let second_server = sync::SourceSyncManager::default();
    let leads = run_leads(&second_server, leads_request(&path, None, LeadMode::Full));

    let listed = sources(&leads);
    assert_eq!(
        listed[0],
        ("py_eval_saved".to_owned(), MechanicSource::ProjectConfig)
    );
    assert!(
        listed.iter().all(|(id, _)| id != "py_eval_session"),
        "a session entry dies with its server: {listed:?}"
    );
    assert_eq!(leads.mechanic_counts[0].count, 4);
}

#[test]
fn persisting_keeps_every_other_config_key() {
    let project = python_project();
    let path = path_of(&project);
    fs::write(
        config_path(&project),
        r#"{"profile": "lean", "default_page_bytes": 4096}"#,
    )
    .expect("write config");
    let manager = sync::SourceSyncManager::default();

    call(&manager, persisted(&path, "py_eval")).expect("persist");

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config_path(&project)).expect("read config"))
            .expect("config stays JSON");
    assert_eq!(written["profile"], "lean");
    assert_eq!(written["default_page_bytes"], 4096);
    assert_eq!(written["mechanics"]["version"], 1);
    assert_eq!(written["mechanics"]["entries"][0]["id"], "py_eval");
    let config = crate::project::ProjectConfig::load(project.path())
        .expect("the limits still parse")
        .expect("config present");
    assert_eq!(config.default_page_bytes, Some(4096));
}

#[cfg(unix)]
#[test]
fn persisting_into_a_symlinked_config_is_refused_and_the_target_is_untouched() {
    let project = python_project();
    let path = path_of(&project);
    let outside = tempfile::tempdir().expect("outside dir");
    let target = outside.path().join("victim.json");
    fs::write(&target, "{}").expect("write target");
    std::os::unix::fs::symlink(&target, config_path(&project)).expect("symlink config");
    let manager = sync::SourceSyncManager::default();

    let error = call(&manager, persisted(&path, "py_eval")).expect_err("a symlink is refused");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error.message.contains("symlink or not a regular file"),
        "{}",
        error.message
    );
    assert_eq!(fs::read_to_string(&target).expect("read target"), "{}");
    let leads = run_leads(&manager, leads_request(&path, None, LeadMode::Full));
    assert!(
        leads
            .mechanic_counts
            .iter()
            .all(|count| count.mechanic != "py_eval"),
        "a refused persist registers nothing"
    );
}

#[test]
fn removing_a_persisted_mechanic_deletes_it_from_config_json() {
    let project = python_project();
    let path = path_of(&project);
    fs::write(config_path(&project), r#"{"profile": "lean"}"#).expect("write config");
    let manager = sync::SourceSyncManager::default();
    call(&manager, persisted(&path, "py_eval")).expect("persist");

    let removed = call(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    )
    .expect("remove");

    assert_eq!(removed.source, Some(MechanicSource::ProjectConfig));
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config_path(&project)).expect("read config"))
            .expect("config stays JSON");
    assert_eq!(written["mechanics"]["entries"], serde_json::json!([]));
    assert_eq!(written["profile"], "lean");
}

#[test]
fn an_invalid_persisted_entry_is_reported_and_counted_while_valid_siblings_load() {
    let project = python_project();
    let path = path_of(&project);
    let valid = serde_json::json!({
        "id": "py_eval", "language": "python", "query": EVAL_QUERY, "note": "eval"
    });
    let section = serde_json::json!({"mechanics": {"version": 1, "entries": [
        valid,
        {"id": "no_note", "language": "python", "query": EVAL_QUERY},
        {"id": "broken", "language": "python", "query": "(call @x", "note": "n"},
        {"id": "unwrap_calls", "language": "python", "query": EVAL_QUERY, "note": "n"},
        {"id": "py_eval", "language": "python", "query": EVAL_QUERY, "note": "again"},
    ]}});
    fs::write(config_path(&project), section.to_string()).expect("write config");
    let manager = sync::SourceSyncManager::default();

    let report = report_of(&manager, &path);
    let leads = run_leads(&manager, leads_request(&path, None, LeadMode::Full));

    assert_eq!(report.mechanics_invalid, 4);
    let invalid: Vec<(Option<usize>, Option<&str>)> = report
        .mechanics
        .invalid
        .iter()
        .map(|failed| (failed.index, failed.id.as_deref()))
        .collect();
    assert_eq!(
        invalid,
        [
            (Some(1), Some("no_note")),
            (Some(2), Some("broken")),
            (Some(3), Some("unwrap_calls")),
            (Some(4), Some("py_eval")),
        ]
    );
    let errors: Vec<&str> = report
        .mechanics
        .invalid
        .iter()
        .map(|failed| failed.error.as_str())
        .collect();
    assert!(errors[0].contains("missing field `note`"), "{errors:?}");
    assert!(errors[1].contains("query does not compile"), "{errors:?}");
    assert!(errors[2].contains("built-in"), "{errors:?}");
    assert!(
        errors[3].contains("earlier persisted mechanic"),
        "{errors:?}"
    );
    assert_eq!(leads.mechanics_invalid, 4);
    assert_eq!(
        sources(&leads)[0],
        ("py_eval".to_owned(), MechanicSource::ProjectConfig)
    );
    assert_eq!(leads.mechanic_counts[0].note.as_deref(), Some("eval"));
    let unwrap = leads
        .mechanic_counts
        .iter()
        .find(|count| count.mechanic == "unwrap_calls")
        .expect("the built-in is still listed");
    assert_eq!(unwrap.source, MechanicSource::Builtin, "never overridden");
}

#[test]
fn an_unknown_mechanics_version_is_reported_invalid_not_ignored() {
    let project = python_project();
    let path = path_of(&project);
    let section = serde_json::json!({"mechanics": {"version": 2, "entries": []}});
    fs::write(config_path(&project), section.to_string()).expect("write config");
    let manager = sync::SourceSyncManager::default();

    let listed = call(&manager, action(&path, MechanicAction::List, None)).expect("list");
    let refused = call(&manager, persisted(&path, "py_eval")).expect_err("not extended");

    assert_eq!(listed.mechanics_invalid, 1);
    assert_eq!(listed.invalid[0].index, None);
    assert!(
        listed.invalid[0]
            .error
            .contains("unsupported mechanics version 2"),
        "{:?}",
        listed.invalid
    );
    assert!(
        refused.message.contains("unsupported mechanics version 2"),
        "{}",
        refused.message
    );
    let written = fs::read_to_string(config_path(&project)).expect("read config");
    assert_eq!(
        written,
        section.to_string(),
        "a future section is not rewritten"
    );
}
