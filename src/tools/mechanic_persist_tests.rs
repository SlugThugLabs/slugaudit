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
    // One manager per process in production: the first server's owner lock
    // (keyed by the shared session id) must be released before a second starts.
    drop(first_server);

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

/// A valid entry at index 0 and, at 1 and 2, two invalid entries no `id`
/// can remove: one with a built-in's id, one with no id at all.
fn config_with_unremovable_by_id(project: &tempfile::TempDir) {
    let section = serde_json::json!({"profile": "lean", "mechanics": {"version": 1, "entries": [
        {"id": "py_eval", "language": "python", "query": EVAL_QUERY, "note": "eval"},
        {"id": "unwrap_calls", "language": "python", "query": EVAL_QUERY, "note": "n"},
        {"language": "python", "query": EVAL_QUERY, "note": "no id"},
    ]}});
    fs::write(config_path(project), section.to_string()).expect("write config");
}

fn remove_at(path: &str, index: usize) -> MechanicRequest {
    let mut request = action(path, MechanicAction::Remove, None);
    request.index = Some(index);
    request
}

#[test]
fn invalid_persisted_entries_no_id_can_name_are_removed_by_their_index() {
    let project = python_project();
    let path = path_of(&project);
    config_with_unremovable_by_id(&project);
    let manager = sync::SourceSyncManager::default();

    let builtin_id = call(&manager, remove_at(&path, 1)).expect("remove the built-in's id");
    let no_id = call(&manager, remove_at(&path, 1)).expect("the id-less entry is now at 1");

    assert_eq!(builtin_id.id.as_deref(), Some("unwrap_calls"));
    assert_eq!(builtin_id.source, Some(MechanicSource::ProjectConfig));
    assert_eq!(no_id.id, None);
    assert_eq!(no_id.mechanics_invalid, 0, "{:?}", no_id.invalid);
    assert_eq!(
        no_id.mechanics[0].id, "py_eval",
        "the valid sibling is untouched"
    );
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config_path(&project)).expect("read config"))
            .expect("config stays JSON");
    assert_eq!(written["profile"], "lean");
    assert_eq!(
        written["mechanics"]["entries"].as_array().map(Vec::len),
        Some(1)
    );
}

#[test]
fn removing_a_valid_persisted_entry_by_index_is_refused_and_the_file_unchanged() {
    let project = python_project();
    let path = path_of(&project);
    config_with_unremovable_by_id(&project);
    let before = fs::read_to_string(config_path(&project)).expect("read config");
    let manager = sync::SourceSyncManager::default();

    let error = call(&manager, remove_at(&path, 0)).expect_err("index 0 loads");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error.message.contains("index 0 is not invalid"),
        "{}",
        error.message
    );
    let after = fs::read_to_string(config_path(&project)).expect("read config");
    assert_eq!(after, before);
}

#[test]
fn a_first_count_with_no_match_has_zero_files_total_and_a_match_counts_its_files() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    let mut no_match = persisted(&path, "py_exec");
    no_match.query = Some(EVAL_QUERY.replace("eval", "exec"));

    let persisted_zero = call(&manager, no_match).expect("persist");
    let session_hit = call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");

    let zero = persisted_zero.first_count.expect("first count");
    assert_eq!(
        (zero.page.total, zero.files_total, zero.files_covered),
        (Some(0), 0, 0),
        "files_total counts files holding a row, not files read"
    );
    let entry = zero
        .mechanic_counts
        .iter()
        .find(|count| count.mechanic == "py_exec")
        .expect("listed");
    assert_eq!(entry.covers, ["python"], "the python files were read");
    let hit = session_hit.first_count.expect("first count");
    assert_eq!((hit.page.total, hit.files_total), (Some(4), 2));
}

#[test]
fn pack_alias_table_matches_the_language_pack() {
    use tree_sitter_language_pack::has_language;
    let resync = "mechanic_catalog::PACK_ALIASES must be re-synced with the language pack's \
                  LANGUAGE_ALIASES (registry.rs)";
    assert!(
        !has_language("no_such_language"),
        "the pack must still refuse an unknown name, or this test proves nothing"
    );
    for (alias, canonical) in mechanic_catalog::PACK_ALIASES {
        assert!(
            has_language(alias),
            "alias `{alias}` is no longer resolved by the pack: {resync}"
        );
        assert!(
            has_language(canonical),
            "canonical `{canonical}` for alias `{alias}` is unknown to the pack: {resync}"
        );
        assert!(
            !mechanic_catalog::PACK_ALIASES
                .iter()
                .any(|(other, _)| other == canonical),
            "canonical `{canonical}` for alias `{alias}` is itself an alias, so it does not \
             resolve to itself: {resync}"
        );
    }
}
