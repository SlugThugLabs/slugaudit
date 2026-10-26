//! The persisted `mechanics` section: read-modify-write keeps every other
//! key, nothing is overwritten that this server cannot read back, and every
//! malformed entry is reported with its position.
use super::*;
use std::fs;

fn project_with_config(contents: Option<&str>) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning/slugaudit")).expect("activation dir");
    if let Some(contents) = contents {
        fs::write(config_path(project.path()), contents).expect("write config");
    }
    project
}

fn entry(id: &str) -> MechanicEntry {
    MechanicEntry {
        id: id.to_owned(),
        language: "python".to_owned(),
        query: "(call) @call".to_owned(),
        note: "every call".to_owned(),
    }
}

#[test]
fn persist_entry_creates_the_section_in_a_missing_config() {
    let project = project_with_config(None);

    persist_entry(project.path(), &entry("calls")).expect("persist");

    let loaded = load_persisted(project.path());
    assert_eq!(loaded.entries, [(0, entry("calls"))]);
    assert!(loaded.invalid.is_empty(), "{:?}", loaded.invalid);
}

#[test]
fn a_config_with_a_mechanics_section_still_applies_its_limits() {
    let project = project_with_config(Some(r#"{"max_parse_bytes": 1234}"#));
    persist_entry(project.path(), &entry("calls")).expect("persist");

    let config = ProjectConfig::load(project.path())
        .expect("the mechanics key is a known key")
        .expect("config present");

    assert_eq!(config.max_parse_bytes, Some(1234));
}

#[test]
fn remove_persisted_of_an_absent_id_is_not_found_and_leaves_the_file_unchanged() {
    let original = r#"{"profile": "lean"}"#;
    let project = project_with_config(Some(original));

    let error = remove_persisted(project.path(), "absent").expect_err("nothing to remove");

    assert!(
        matches!(error, MechanicsWriteError::NotFound(ref id) if id == "absent"),
        "{error}"
    );
    let written = fs::read_to_string(config_path(project.path())).expect("read config");
    assert_eq!(written, original);
}

#[test]
fn persist_entry_refuses_a_config_that_is_not_json_and_leaves_it_unchanged() {
    let project = project_with_config(Some("{ not json"));

    let error = persist_entry(project.path(), &entry("calls")).expect_err("not overwritten");

    assert!(
        matches!(error, MechanicsWriteError::Read(ConfigError::Parse { .. })),
        "{error}"
    );
    let written = fs::read_to_string(config_path(project.path())).expect("read config");
    assert_eq!(written, "{ not json");
}

#[test]
fn persist_entry_refuses_past_the_persisted_cap() {
    let project = project_with_config(None);
    for index in 0..MAX_PERSISTED_MECHANICS {
        persist_entry(project.path(), &entry(&format!("m{index}"))).expect("under the cap");
    }

    let error = persist_entry(project.path(), &entry("one_more")).expect_err("over the cap");

    assert!(matches!(error, MechanicsWriteError::Full), "{error}");
    assert_eq!(
        load_persisted(project.path()).entries.len(),
        MAX_PERSISTED_MECHANICS
    );
}

#[test]
fn load_persisted_reports_a_malformed_entry_with_its_index_and_keeps_the_rest() {
    let project = project_with_config(Some(
        r#"{"mechanics": {"version": 1, "entries": [
            {"id": "calls", "language": "python", "query": "(call) @call", "note": "every call"},
            {"id": "extra", "language": "python", "query": "(call) @c", "note": "n", "x": 1},
            "not an object"
        ]}}"#,
    ));

    let loaded = load_persisted(project.path());

    assert_eq!(loaded.entries, [(0, entry("calls"))]);
    let reported: Vec<(Option<usize>, Option<&str>)> = loaded
        .invalid
        .iter()
        .map(|failed| (failed.index, failed.id.as_deref()))
        .collect();
    assert_eq!(reported, [(Some(1), Some("extra")), (Some(2), None)]);
    assert!(
        loaded.invalid[0].error.contains("unknown field `x`"),
        "{:?}",
        loaded.invalid
    );
}

#[test]
fn load_persisted_reports_a_section_without_a_version_as_one_invalid_section() {
    let project = project_with_config(Some(r#"{"mechanics": {"entries": []}}"#));

    let loaded = load_persisted(project.path());

    assert!(loaded.entries.is_empty());
    assert_eq!(loaded.invalid.len(), 1);
    assert_eq!(loaded.invalid[0].index, None);
    assert!(
        loaded.invalid[0].error.contains("version"),
        "{:?}",
        loaded.invalid
    );
}
