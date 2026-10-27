//! Tests for the project config: load and limit mapping (the write path is
//! tested in `config_store_tests.rs`).

use super::*;
use crate::store::test_capture::capture_warns;

#[test]
fn loads_valid_config_and_applies_overrides() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conf_dir = dir.path().join(".planning").join("slugaudit");
    std::fs::create_dir_all(&conf_dir).expect("create dir");
    let json = r#"{"profile": "audit", "max_file_bytes": 536870912}"#;
    std::fs::write(conf_dir.join("config.json"), json).expect("write");

    let loaded = ProjectConfig::load(dir.path()).expect("load should succeed");
    assert!(loaded.is_some());

    let cfg = ProjectConfig::load_or_default(dir.path());
    assert_eq!(cfg.profile, Some(AuditProfile::Audit));
    assert_eq!(cfg.max_file_bytes, Some(536_870_912));

    let base = ResourceLimits::default();
    let applied = cfg.apply_to_limits(base);
    assert_eq!(applied.max_file_bytes, 536_870_912);
    assert_eq!(applied.max_total_import_bytes, 32 * 1024 * 1024 * 1024);
}

#[test]
fn loads_default_when_missing_or_invalid() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(
        ProjectConfig::load(dir.path())
            .expect("load missing")
            .is_none()
    );
    let cfg = ProjectConfig::load_or_default(dir.path());
    assert!(cfg.profile.is_none());
    assert!(cfg.max_file_bytes.is_none());

    let conf_dir = dir.path().join(".planning").join("slugaudit");
    std::fs::create_dir_all(&conf_dir).expect("create dir");
    let conf_file = conf_dir.join("config.json");
    std::fs::write(&conf_file, "invalid json").expect("write");

    let load_err = ProjectConfig::load(dir.path()).expect_err("should return Parse error");
    match load_err {
        ConfigError::Parse { path, .. } => assert_eq!(path, conf_file),
        ConfigError::Io { .. } => panic!("expected Parse error, got Io"),
    }

    let cfg_invalid = ProjectConfig::load_or_default(dir.path());
    assert!(cfg_invalid.profile.is_none());

    let custom = ProjectConfig {
        max_total_import_bytes: Some(1024),
        ..Default::default()
    };
    let applied = custom.apply_to_limits(ResourceLimits::default());
    assert_eq!(applied.max_total_import_bytes, 1024);
}

#[test]
fn a_configured_default_page_size_overrides_the_profile_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conf_dir = dir.path().join(".planning").join("slugaudit");
    std::fs::create_dir_all(&conf_dir).expect("create dir");
    std::fs::write(
        conf_dir.join("config.json"),
        r#"{"profile": "lean", "default_page_bytes": 4096}"#,
    )
    .expect("write");

    let cfg = ProjectConfig::load_or_default(dir.path());
    let applied = cfg.apply_to_limits(ResourceLimits::default());

    assert_eq!(applied.default_page_bytes, 4_096);
    assert_eq!(applied.max_query_response_bytes, 32 * 1024 * 1024);
}

#[test]
fn a_misspelled_config_key_is_a_parse_error_not_silently_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conf_dir = dir.path().join(".planning").join("slugaudit");
    std::fs::create_dir_all(&conf_dir).expect("create dir");
    std::fs::write(conf_dir.join("config.json"), r#"{"max_file_byte": 5}"#).expect("write");

    let error = ProjectConfig::load(dir.path()).expect_err("unknown key must fail");

    assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
}

#[test]
fn an_uninspectable_config_path_is_an_io_error_not_no_config() {
    let root = Path::new("/tmp").join("a".repeat(300));

    let error = ProjectConfig::load(&root).expect_err("stat failure must surface");

    assert!(matches!(error, ConfigError::Io { .. }), "{error}");
}

/// R6: an I/O failure (here a directory at the config path) is not a
/// parse failure, and the log must send the operator to the right fix.
#[test]
fn an_unreadable_config_is_not_reported_as_malformed() {
    let directory = tempfile::tempdir().expect("tempdir");
    let config = directory.path().join(".planning/slugaudit/config.json");
    std::fs::create_dir_all(config).expect("dir");
    let (_, logs) = capture_warns(|| ProjectConfig::load_or_default(directory.path()));
    assert!(
        logs.contains("could not be read") && !logs.contains("malformed"),
        "{logs}"
    );
}

#[test]
fn a_config_file_above_sqlite_max_is_clamped() {
    let config = ProjectConfig {
        max_file_bytes: Some(2_000_000_000),
        ..Default::default()
    };

    let applied = config.apply_to_limits(ResourceLimits::default());

    assert_eq!(
        applied.max_file_bytes,
        crate::model::SQLITE_MAX_VALUE_BYTES,
        "SQLite cannot store a larger content value; the project limit is clamped"
    );
}

#[test]
fn a_configured_max_parse_bytes_overrides_the_profile_default() {
    let config = ProjectConfig {
        max_parse_bytes: Some(4_096),
        ..Default::default()
    };

    let applied = config.apply_to_limits(ResourceLimits::default());

    assert_eq!(applied.max_parse_bytes, 4_096);
}

#[test]
fn a_configured_max_parse_bytes_above_max_file_bytes_is_clamped_and_reported() {
    let config = ProjectConfig {
        max_file_bytes: Some(10_000),
        max_parse_bytes: Some(20_000),
        ..Default::default()
    };

    let (applied, logs) = capture_warns(|| config.apply_to_limits(ResourceLimits::default()));

    assert_eq!(applied.max_parse_bytes, 10_000);
    assert!(
        logs.contains("max_parse_bytes exceeds max_file_bytes; clamping"),
        "{logs}"
    );
}

#[test]
fn a_configured_max_parse_wall_clock_ms_overrides_the_profile_default() {
    let config = ProjectConfig {
        max_parse_wall_clock_ms: Some(250),
        ..Default::default()
    };

    let applied = config.apply_to_limits(ResourceLimits::default());

    assert_eq!(applied.max_parse_wall_clock_ms, 250);
}

#[test]
fn a_configured_max_structure_query_bytes_overrides_the_profile_default() {
    let config = ProjectConfig {
        max_structure_query_bytes: Some(512),
        ..Default::default()
    };

    let applied = config.apply_to_limits(ResourceLimits::default());

    assert_eq!(applied.max_structure_query_bytes, 512);
}

#[test]
fn a_non_numeric_max_parse_bytes_is_a_parse_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conf_dir = dir.path().join(".planning").join("slugaudit");
    std::fs::create_dir_all(&conf_dir).expect("create dir");
    std::fs::write(
        conf_dir.join("config.json"),
        r#"{"max_parse_bytes": "big"}"#,
    )
    .expect("write");

    let error = ProjectConfig::load(dir.path()).expect_err("a string is not a byte count");

    assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
}

#[test]
fn a_configured_result_store_budget_and_grace_override_the_profile_defaults() {
    let config = ProjectConfig {
        max_result_store_bytes: Some(777),
        result_active_grace_secs: Some(33),
        ..Default::default()
    };

    let applied = config.apply_to_limits(ResourceLimits::default());

    assert_eq!(applied.max_result_store_bytes, 777);
    assert_eq!(applied.result_active_grace_secs, 33);
}
