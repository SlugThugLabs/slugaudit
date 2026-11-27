//! Tests for multi-source limit precedence, clamping, warn-once ledger, and
//! startup/project limit entrypoints.

use super::*;
use crate::model::{SQLITE_MAX_VALUE_BYTES, fresh_ledger};
use crate::project::global_config::tests::GlobalConfigGuard;
use crate::util::test_capture::capture_warns;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::time::Duration;

fn lines_containing(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
}

fn resolve_twice_with_warns(config: &GlobalConfig) -> TestResult<(ResourceLimits, String)> {
    fresh_ledger().check("reset warning ledger")?;
    let (resolved, logs) = capture_warns(|| -> TestResult<ResourceLimits> {
        let first = resolve_limits(config, &EnvOverrides::default(), Some(AuditProfile::Lean))
            .check("first resolve")?;
        let second = resolve_limits(config, &EnvOverrides::default(), Some(AuditProfile::Lean))
            .check("second resolve")?;
        ensure_eq(&first, &second, "repeated resolution is deterministic")?;
        Ok(second)
    })?;
    Ok((resolved?, logs))
}

fn assert_zero_key_error(error: &ProjectLimitsError, expected_key: &'static str) -> TestResult {
    ensure(
        matches!(
            error,
            ProjectLimitsError::Config(ConfigError::Zero { key, .. }) if *key == expected_key
        ),
        format!("expected ConfigError::Zero for {expected_key}, got {error:?}"),
    )
}

#[test]
fn a_valid_global_config_is_applied_across_all_limit_fields() -> TestResult {
    let _guard =
        GlobalConfigGuard::with_contents(r#"{"profile": "audit", "max_file_bytes": 536870912}"#)?;
    let audit_limits = resolve_limits(
        &GlobalConfig::load().check("loads")?,
        &EnvOverrides::default(),
        None,
    )
    .check("limits resolve")?;
    ensure_eq(
        &audit_limits.max_file_bytes,
        &536_870_912,
        "global config overrides max_file_bytes",
    )?;
    ensure_eq(
        &audit_limits.max_total_import_bytes,
        &(32 * 1024 * 1024 * 1024),
        "global config audit profile sets 32 GiB import budget",
    )?;

    let custom = GlobalConfig {
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
    };
    let resolved = resolve_limits(&custom, &EnvOverrides::default(), None).check("all 16")?;
    let expected = ResourceLimits {
        max_file_bytes: 4_194_304,
        max_parse_bytes: 1_048_576,
        max_parse_wall_clock_ms: 2_500,
        max_total_import_bytes: 67_108_864,
        max_query_response_bytes: 2_097_152,
        default_page_bytes: 16_384,
        max_query_sql_bytes: 25_000,
        max_query_vm_steps: 1_200_000,
        max_query_wall_clock: Duration::from_secs(15),
        max_query_value_bytes: 524_288,
        max_structure_query_bytes: 8_192,
        max_structure_matches: 1_500,
        max_structure_execution_time: Duration::from_secs(20),
        max_sync_wall_clock: Duration::from_secs(45),
        max_result_store_bytes: 16_777_216,
        result_active_grace_secs: 300,
        ..limits_for_profile(AuditProfile::Lean).check("lean base")?
    };
    ensure_eq(
        &resolved,
        &expected,
        "all 16 GlobalConfig limit keys apply to ResourceLimits",
    )
}

#[test]
fn fail_closed_zero_limits_are_rejected_in_resolve_effective_and_startup() -> TestResult {
    let in_mem = GlobalConfig {
        max_sync_wall_clock_secs: Some(0),
        ..Default::default()
    };
    let mem_err = resolve_limits(&in_mem, &EnvOverrides::default(), Some(AuditProfile::Lean))
        .check_err("an in-memory zero limit must fail resolve_limits")?;
    assert_zero_key_error(&mem_err, "max_sync_wall_clock_secs")?;

    let guard = GlobalConfigGuard::with_contents(r#"{"max_file_bytes": 0}"#)?;
    let eff_err = effective_limits(Path::new("/precedence-test/invalid")).check_err("must fail")?;
    assert_zero_key_error(&eff_err, "max_file_bytes")?;
    let start_err = validate_startup_config().check_err("startup validation must fail")?;
    assert_zero_key_error(&start_err, "max_file_bytes")?;
    drop(guard);

    let _env_guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    validate_startup_config().check("clean startup validation succeeds")?;
    Ok(())
}

#[test]
fn a_per_repo_config_file_is_ignored() -> TestResult {
    let _env_guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let _cfg_guard = GlobalConfigGuard::with_path(None);
    let project = tempfile::tempdir().check("project")?;
    let activation = project.path().join(".planning/slugaudit");
    std::fs::create_dir_all(&activation).check("activation dir")?;
    std::fs::write(activation.join("config.json"), "{ not even json").check("write")?;

    let limits = effective_limits(project.path()).check("the per-repo file is never read")?;
    let expected = resolve_limits(
        &GlobalConfig::default(),
        &env_overrides().check("env")?,
        None,
    )
    .check("limits")?;
    ensure_eq(
        &limits.default_page_bytes,
        &expected.default_page_bytes,
        "per-repo config.json is ignored in favor of global and environment settings",
    )
}

#[test]
fn precedence_orders_session_profile_then_env_then_global_file_then_defaults() -> TestResult {
    let global = GlobalConfig {
        profile: Some(AuditProfile::Audit),
        max_file_bytes: Some(1_000_000),
        default_page_bytes: Some(4_096),
        ..Default::default()
    };
    let env = EnvOverrides {
        profile: Some(AuditProfile::Audit),
        max_file_bytes: Some(2_000_000),
        ..Default::default()
    };

    let with_session =
        resolve_limits(&global, &env, Some(AuditProfile::Lean)).check("session resolve")?;
    let lean = limits_for_profile(AuditProfile::Lean).check("lean")?;
    ensure_eq(
        &with_session.max_total_import_bytes,
        &lean.max_total_import_bytes,
        "session profile beats env and file profiles for base table",
    )?;
    ensure_eq(
        &with_session.max_file_bytes,
        &2_000_000,
        "env limit overrides global file limit",
    )?;
    ensure_eq(
        &with_session.default_page_bytes,
        &4_096,
        "global file limit overrides profile default",
    )?;

    let env_lean = EnvOverrides {
        profile: Some(AuditProfile::Lean),
        ..Default::default()
    };
    let without_session = resolve_limits(&global, &env_lean, None).check("env profile resolve")?;
    ensure_eq(
        &without_session.max_total_import_bytes,
        &lean.max_total_import_bytes,
        "SLUGAUDIT_PROFILE overrides config.json profile when session profile is unset",
    )
}

#[test]
fn an_environment_response_ceiling_moves_the_result_store_budget_unless_the_file_pinned_it()
-> TestResult {
    let env = EnvOverrides {
        max_query_response_bytes: Some(1_000_000),
        ..Default::default()
    };
    let derived = resolve_limits(&GlobalConfig::default(), &env, Some(AuditProfile::Lean))
        .check("derived resolve")?;
    let pinned = resolve_limits(
        &GlobalConfig {
            max_result_store_bytes: Some(777),
            ..Default::default()
        },
        &env,
        Some(AuditProfile::Lean),
    )
    .check("pinned resolve")?;

    ensure_eq(
        &derived.max_result_store_bytes,
        &4_000_000,
        "unpinned result store budget scales with overridden response ceiling",
    )?;
    ensure_eq(
        &pinned.max_result_store_bytes,
        &777,
        "config.json max_result_store_bytes pins the result store budget",
    )
}

#[test]
fn the_real_environment_overrides_the_global_file_end_to_end() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("3000000")),
        ],
        || -> TestResult {
            let env = EnvOverrides::read_env().check("valid env")?;
            let global = GlobalConfig {
                max_file_bytes: Some(1_000_000),
                ..Default::default()
            };
            let limits = resolve_limits(&global, &env, None).check("limits resolve")?;
            ensure_eq(
                &limits.max_file_bytes,
                &3_000_000,
                "environment variable overrides config.json max_file_bytes",
            )
        },
    )
}

#[test]
fn config_clamping_warns_once_and_respects_higher_precedence_env_file_bytes() -> TestResult {
    let over_sqlite = GlobalConfig {
        max_file_bytes: Some(2_000_000_000),
        max_query_value_bytes: Some(2_000_000_000),
        ..Default::default()
    };
    let (limits, logs) = resolve_twice_with_warns(&over_sqlite)?;
    ensure_eq(
        &limits.max_file_bytes,
        &SQLITE_MAX_VALUE_BYTES,
        "config max_file_bytes clamps to SQLITE_MAX_VALUE_BYTES",
    )?;
    ensure_eq(
        &limits.max_query_value_bytes,
        &usize::try_from(SQLITE_MAX_VALUE_BYTES).check("cap fits usize")?,
        "config max_query_value_bytes clamps to SQLITE_MAX_VALUE_BYTES",
    )?;
    ensure_eq(
        &lines_containing(&logs, "setting=\"config max_file_bytes\""),
        &1,
        &logs,
    )?;
    ensure_eq(
        &lines_containing(&logs, "setting=\"config max_query_value_bytes\""),
        &1,
        &logs,
    )?;

    let parse_over_file = GlobalConfig {
        max_file_bytes: Some(10_000),
        max_parse_bytes: Some(20_000),
        ..Default::default()
    };
    let (clamped, parse_logs) = resolve_twice_with_warns(&parse_over_file)?;
    ensure_eq(
        &clamped.max_parse_bytes,
        &10_000,
        "max_parse_bytes clamps to configured max_file_bytes",
    )?;
    ensure_eq(
        &lines_containing(&parse_logs, "requested=20000"),
        &1,
        &parse_logs,
    )?;

    let env_raises = EnvOverrides {
        max_file_bytes: Some(10_000_000),
        ..Default::default()
    };
    let unclamped = resolve_limits(
        &GlobalConfig {
            max_file_bytes: Some(10_000),
            ..Default::default()
        },
        &env_raises,
        Some(AuditProfile::Lean),
    )
    .check("resolve with higher env max_file_bytes")?;
    ensure_eq(
        &unclamped.max_parse_bytes,
        &(2 * 1024 * 1024),
        "higher-precedence SLUGAUDIT_MAX_FILE_BYTES prevents config.json from clamping max_parse_bytes",
    )
}
