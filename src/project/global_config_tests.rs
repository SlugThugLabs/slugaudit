//! Tests for the global config: load, the limit keys, and the precedence of
//! defaults, the global file, the environment and the session profile.

use super::*;
use crate::store::test_capture::capture_warns;

/// A temp dir holding `config.json` with `contents`, and the file's path.
fn config_file(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.json");
    std::fs::write(&path, contents).expect("write config");
    (dir, path)
}

/// Points this thread's global config at `contents`.
fn use_global_config(contents: &str) -> tempfile::TempDir {
    let (dir, path) = config_file(contents);
    set_global_config_path_for_test(Some(path));
    dir
}

#[test]
fn an_absent_global_config_is_the_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("config.json");

    assert!(
        GlobalConfig::load_from(&missing)
            .expect("absent is fine")
            .is_none()
    );
    set_global_config_path_for_test(Some(missing));
    let config = GlobalConfig::load().expect("absent is the default");
    assert!(config.profile.is_none());
    assert!(config.max_file_bytes.is_none());
    set_global_config_path_for_test(None);
    assert!(
        GlobalConfig::load()
            .expect("no home is the default")
            .profile
            .is_none()
    );
}

#[test]
fn a_valid_global_config_is_applied() {
    let _dir = use_global_config(r#"{"profile": "audit", "max_file_bytes": 536870912}"#);

    let limits = resolve_limits(
        &GlobalConfig::load().expect("loads"),
        &EnvOverrides::default(),
        None,
    )
    .expect("limits resolve");

    assert_eq!(limits.max_file_bytes, 536_870_912);
    assert_eq!(limits.max_total_import_bytes, 32 * 1024 * 1024 * 1024);
}

#[test]
fn invalid_json_fails_the_load_naming_the_file() {
    let (_dir, path) = config_file("invalid json");

    let error = GlobalConfig::load_from(&path).expect_err("malformed must fail");

    assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
    assert!(
        error.to_string().contains(path.to_str().expect("utf-8")),
        "{error}"
    );
}

#[test]
fn an_unknown_key_fails_the_load_naming_the_key() {
    let (_dir, path) = config_file(r#"{"max_file_byte": 5}"#);

    let error = GlobalConfig::load_from(&path).expect_err("unknown key must fail");

    assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
    assert!(error.to_string().contains("max_file_byte"), "{error}");
    assert!(
        error.to_string().contains(path.to_str().expect("utf-8")),
        "{error}"
    );
}

#[test]
fn a_mechanics_key_is_an_unknown_key() {
    let (_dir, path) = config_file(r#"{"mechanics": {"version": 1, "entries": []}}"#);

    let error = GlobalConfig::load_from(&path).expect_err("mechanics are session-only");

    assert!(
        error.to_string().contains("unknown field `mechanics`"),
        "{error}"
    );
}

#[test]
fn an_unreadable_config_is_an_io_error_naming_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).expect("a directory at the config path");

    let error = GlobalConfig::load_from(&path).expect_err("unreadable must fail");

    assert!(matches!(error, ConfigError::Io { .. }), "{error}");
    assert!(
        error.to_string().contains(path.to_str().expect("utf-8")),
        "{error}"
    );
}

#[test]
fn effective_limits_fails_for_an_invalid_global_config_instead_of_using_defaults() {
    let _dir = use_global_config("{");

    let error = effective_limits(Path::new("/precedence-test/invalid")).expect_err("must fail");

    assert!(
        matches!(error, ProjectLimitsError::Config(ConfigError::Parse { .. })),
        "{error}"
    );
}

#[test]
fn a_per_repo_config_file_is_ignored() {
    let project = tempfile::tempdir().expect("project");
    let activation = project.path().join(".planning/slugaudit");
    std::fs::create_dir_all(&activation).expect("activation dir");
    std::fs::write(activation.join("config.json"), "{ not even json").expect("write");
    set_global_config_path_for_test(None);

    let limits = effective_limits(project.path()).expect("the per-repo file is never read");

    assert_eq!(
        limits.default_page_bytes,
        resolve_limits(
            &GlobalConfig::default(),
            env_overrides().expect("env"),
            None
        )
        .expect("limits")
        .default_page_bytes
    );
}

#[test]
fn the_environment_beats_the_global_file() {
    let global = GlobalConfig {
        max_file_bytes: Some(1_000_000),
        default_page_bytes: Some(4_096),
        ..Default::default()
    };
    let env = EnvOverrides {
        max_file_bytes: Some(2_000_000),
        ..Default::default()
    };

    let limits = resolve_limits(&global, &env, None).expect("limits resolve");

    assert_eq!(limits.max_file_bytes, 2_000_000, "env beats the file");
    assert_eq!(
        limits.default_page_bytes, 4_096,
        "the file beats the default"
    );
}

#[test]
fn the_session_profile_picks_the_base_and_keeps_file_and_env_limits() {
    let global = GlobalConfig {
        profile: Some(AuditProfile::Audit),
        default_page_bytes: Some(4_096),
        ..Default::default()
    };
    let env = EnvOverrides {
        profile: Some(AuditProfile::Audit),
        max_file_bytes: Some(2_000_000),
        ..Default::default()
    };

    let limits = resolve_limits(&global, &env, Some(AuditProfile::Lean)).expect("limits resolve");

    let lean = limits_for_profile(AuditProfile::Lean).expect("lean");
    assert_eq!(
        limits.max_total_import_bytes, lean.max_total_import_bytes,
        "the session profile beats the env and file profiles for the base"
    );
    assert_eq!(
        limits.max_file_bytes, 2_000_000,
        "choosing a profile keeps the env limit"
    );
    assert_eq!(
        limits.default_page_bytes, 4_096,
        "choosing a profile keeps the file limit"
    );
}

#[test]
fn the_env_profile_beats_the_file_profile_and_the_default() {
    let global = GlobalConfig {
        profile: Some(AuditProfile::Audit),
        ..Default::default()
    };
    let env = EnvOverrides {
        profile: Some(AuditProfile::Lean),
        ..Default::default()
    };

    let limits = resolve_limits(&global, &env, None).expect("limits resolve");

    assert_eq!(
        limits.max_total_import_bytes,
        limits_for_profile(AuditProfile::Lean)
            .expect("lean")
            .max_total_import_bytes
    );
}

#[test]
fn an_environment_response_ceiling_moves_the_result_store_budget_unless_the_file_pinned_it() {
    let env = EnvOverrides {
        max_query_response_bytes: Some(1_000_000),
        ..Default::default()
    };

    let derived = resolve_limits(&GlobalConfig::default(), &env, Some(AuditProfile::Lean))
        .expect("limits resolve");
    let pinned = resolve_limits(
        &GlobalConfig {
            max_result_store_bytes: Some(777),
            ..Default::default()
        },
        &env,
        Some(AuditProfile::Lean),
    )
    .expect("limits resolve");

    assert_eq!(derived.max_result_store_bytes, 4_000_000);
    assert_eq!(pinned.max_result_store_bytes, 777);
}

#[test]
fn the_real_environment_overrides_the_global_file_end_to_end() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("3000000")),
        ],
        || {
            let env = EnvOverrides::read().expect("valid env");
            let global = GlobalConfig {
                max_file_bytes: Some(1_000_000),
                ..Default::default()
            };

            let limits = resolve_limits(&global, &env, None).expect("limits resolve");

            assert_eq!(limits.max_file_bytes, 3_000_000);
        },
    );
}

#[test]
fn a_configured_default_page_size_overrides_the_profile_default() {
    let (_dir, path) = config_file(r#"{"profile": "lean", "default_page_bytes": 4096}"#);

    let cfg = GlobalConfig::load_from(&path)
        .expect("config loads")
        .expect("config present");
    let applied = cfg.apply_limit_keys(limits_for_profile(AuditProfile::Lean).expect("lean"));

    assert_eq!(applied.default_page_bytes, 4_096);
    assert_eq!(applied.max_query_response_bytes, 32 * 1024 * 1024);
}

#[test]
fn a_config_file_above_sqlite_max_is_clamped() {
    let config = GlobalConfig {
        max_file_bytes: Some(2_000_000_000),
        ..Default::default()
    };

    let applied = config.apply_limit_keys(ResourceLimits::default());

    assert_eq!(
        applied.max_file_bytes,
        crate::model::SQLITE_MAX_VALUE_BYTES,
        "SQLite cannot store a larger content value; the project limit is clamped"
    );
}

#[test]
fn a_configured_max_parse_bytes_overrides_the_profile_default() {
    let config = GlobalConfig {
        max_parse_bytes: Some(4_096),
        ..Default::default()
    };

    let applied = config.apply_limit_keys(ResourceLimits::default());

    assert_eq!(applied.max_parse_bytes, 4_096);
}

#[test]
fn a_configured_max_parse_bytes_above_max_file_bytes_is_clamped_and_reported() {
    let config = GlobalConfig {
        max_file_bytes: Some(10_000),
        max_parse_bytes: Some(20_000),
        ..Default::default()
    };

    let (applied, logs) = capture_warns(|| config.apply_limit_keys(ResourceLimits::default()));

    assert_eq!(applied.max_parse_bytes, 10_000);
    assert!(
        logs.contains("max_parse_bytes exceeds max_file_bytes; clamping"),
        "{logs}"
    );
}

#[test]
fn a_configured_max_parse_wall_clock_ms_overrides_the_profile_default() {
    let config = GlobalConfig {
        max_parse_wall_clock_ms: Some(250),
        ..Default::default()
    };

    let applied = config.apply_limit_keys(ResourceLimits::default());

    assert_eq!(applied.max_parse_wall_clock_ms, 250);
}

#[test]
fn a_configured_max_structure_query_bytes_overrides_the_profile_default() {
    let config = GlobalConfig {
        max_structure_query_bytes: Some(512),
        ..Default::default()
    };

    let applied = config.apply_limit_keys(ResourceLimits::default());

    assert_eq!(applied.max_structure_query_bytes, 512);
}

#[test]
fn a_non_numeric_max_parse_bytes_is_a_parse_error() {
    let (_dir, path) = config_file(r#"{"max_parse_bytes": "big"}"#);

    let error = GlobalConfig::load_from(&path).expect_err("a string is not a byte count");

    assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
}

#[test]
fn a_configured_result_store_budget_and_grace_override_the_profile_defaults() {
    let config = GlobalConfig {
        max_result_store_bytes: Some(777),
        result_active_grace_secs: Some(33),
        ..Default::default()
    };

    let applied = config.apply_limit_keys(ResourceLimits::default());

    assert_eq!(applied.max_result_store_bytes, 777);
    assert_eq!(applied.result_active_grace_secs, 33);
}
