//! Tests for `GlobalConfig` schema, capped disk I/O, and load-time validation.

use super::*;
use crate::model::AuditProfile;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

/// RAII guard that points the calling thread's global config at a temporary
/// file and resets `set_global_config_path_for_test(None)` on drop.
pub(in crate::project) struct GlobalConfigGuard {
    _dir: Option<tempfile::TempDir>,
}

impl GlobalConfigGuard {
    pub(in crate::project) fn with_contents(contents: &str) -> TestResult<Self> {
        let (dir, path) = config_file(contents)?;
        set_global_config_path_for_test(Some(path));
        Ok(Self { _dir: Some(dir) })
    }

    pub(in crate::project) fn with_path(path: Option<PathBuf>) -> Self {
        set_global_config_path_for_test(path);
        Self { _dir: None }
    }
}

impl Drop for GlobalConfigGuard {
    fn drop(&mut self) {
        set_global_config_path_for_test(None);
    }
}

/// A temp dir holding `config.json` with `contents`, and the file's path.
fn config_file(contents: &str) -> TestResult<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("config.json");
    std::fs::write(&path, contents).check("write config")?;
    Ok((dir, path))
}

const ALL_LIMIT_SAMPLES: [(&str, u64); 16] = [
    ("max_file_bytes", 4_194_304),
    ("max_parse_bytes", 1_048_576),
    ("max_parse_wall_clock_ms", 2_500),
    ("max_total_import_bytes", 67_108_864),
    ("max_query_response_bytes", 2_097_152),
    ("default_page_bytes", 16_384),
    ("max_query_sql_bytes", 25_000),
    ("max_query_vm_steps", 1_200_000),
    ("max_query_wall_clock_secs", 15),
    ("max_query_value_bytes", 524_288),
    ("max_structure_query_bytes", 8_192),
    ("max_structure_matches", 1_500),
    ("max_structure_execution_time_secs", 20),
    ("max_sync_wall_clock_secs", 45),
    ("max_result_store_bytes", 16_777_216),
    ("result_active_grace_secs", 300),
];

#[test]
fn an_absent_global_config_is_the_default() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let missing = dir.path().join("config.json");

    ensure(
        GlobalConfig::load_from(&missing)
            .check("absent is fine")?
            .is_none(),
        "missing config file returns None from load_from",
    )?;
    let guard = GlobalConfigGuard::with_path(Some(missing));
    let config = GlobalConfig::load().check("absent is the default")?;
    ensure_eq(
        &config,
        &GlobalConfig::default(),
        "absent config path resolves to GlobalConfig::default()",
    )?;
    drop(guard);
    ensure_eq(
        &GlobalConfig::load().check("no home is the default")?,
        &GlobalConfig::default(),
        "unset home path resolves to GlobalConfig::default()",
    )?;
    Ok(())
}

#[test]
fn all_sixteen_documented_limit_keys_deserialize_cleanly() -> TestResult {
    let mut entries: Vec<String> = vec![r#""profile": "lean""#.to_owned()];
    entries.extend(
        ALL_LIMIT_SAMPLES
            .iter()
            .map(|(key, val)| format!(r#""{key}": {val}"#)),
    );
    let json = format!("{{{}}}", entries.join(", "));
    let (_dir, path) = config_file(&json)?;
    let cfg = GlobalConfig::load_from(&path)
        .check("config loads")?
        .check("config present")?;

    ensure_eq(
        &cfg,
        &GlobalConfig {
            profile: Some(AuditProfile::Lean),
            max_file_bytes: Some(4_194_304),
            max_parse_bytes: Some(1_048_576),
            max_parse_wall_clock_ms: Some(2_500),
            max_total_import_bytes: Some(67_108_864),
            max_query_response_bytes: Some(2_097_152),
            default_page_bytes: Some(16_384),
            max_query_sql_bytes: Some(25_000),
            max_query_vm_steps: Some(1_200_000),
            max_query_wall_clock_secs: Some(15),
            max_query_value_bytes: Some(524_288),
            max_structure_query_bytes: Some(8_192),
            max_structure_matches: Some(1_500),
            max_structure_execution_time_secs: Some(20),
            max_sync_wall_clock_secs: Some(45),
            max_result_store_bytes: Some(16_777_216),
            result_active_grace_secs: Some(300),
        },
        "all 16 limit keys and profile deserialize to exact GlobalConfig fields",
    )
}

#[test]
fn fail_closed_any_zero_limit_in_config_json_fails_naming_the_key() -> TestResult {
    for (key, _) in ALL_LIMIT_SAMPLES {
        let json = format!(r#"{{"{key}": 0}}"#);
        let (_dir, path) = config_file(&json)?;
        let error =
            GlobalConfig::load_from(&path).check_err("a zero limit in config.json must fail")?;
        ensure(
            matches!(error, ConfigError::Zero { key: named, path: Some(_) } if named == key),
            format!("expected ConfigError::Zero naming {key}, got {error:?}"),
        )?;
        let message = error.to_string();
        ensure(
            message.contains(key) && message.contains("is set to 0"),
            format!("error message must name {key} and state 0 is unusable: {message}"),
        )?;
    }
    Ok(())
}

fn assert_parse_error(contents: &str, expected_substring: Option<&str>) -> TestResult {
    let (_dir, path) = config_file(contents)?;
    let error = GlobalConfig::load_from(&path).check_err("invalid config JSON must fail")?;
    ensure(
        matches!(error, ConfigError::Parse { .. }),
        format!("expected ConfigError::Parse, got {error:?}"),
    )?;
    let msg = error.to_string();
    ensure(
        msg.contains(path.to_str().check("utf-8 path")?),
        format!("parse error must name file path: {msg}"),
    )?;
    if let Some(needle) = expected_substring {
        ensure(
            msg.contains(needle),
            format!("parse error must contain '{needle}': {msg}"),
        )?;
    }
    Ok(())
}

#[test]
fn malformed_unknown_and_wrongly_typed_json_fail_with_parse_errors() -> TestResult {
    assert_parse_error("invalid json", None)?;
    assert_parse_error(r#"{"max_file_byte": 5}"#, Some("max_file_byte"))?;
    assert_parse_error(
        r#"{"mechanics": {"version": 1, "entries": []}}"#,
        Some("unknown field `mechanics`"),
    )?;
    assert_parse_error(r#"{"max_parse_bytes": "big"}"#, None)
}

#[test]
fn an_unreadable_config_is_an_io_error_naming_the_file() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).check("a directory at the config path")?;

    let error = GlobalConfig::load_from(&path).check_err("unreadable must fail")?;

    ensure(
        matches!(error, ConfigError::Io { .. }),
        format!("expected ConfigError::Io, got {error:?}"),
    )?;
    ensure(
        error.to_string().contains(path.to_str().check("utf-8")?),
        format!("I/O error must name file path: {error}"),
    )?;
    Ok(())
}
