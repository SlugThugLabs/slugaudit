//! Fail-closed startup validation and profile fallback behavior against the
//! live `project::validate_startup_config` / `resolve_limits_with` entrypoints.

use super::*;

#[test]
fn fail_closed_an_invalid_limit_variable_is_an_error_naming_it_not_its_value() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("12MBbadtoken")),
        ],
        || {
            let error = startup_limits_from_env().check_err("a typo'd limit must fail startup")?;
            let message = error.to_string();
            ensure(
                message.starts_with(
                    "SLUGAUDIT_MAX_FILE_BYTES is set but is not valid; expected a value of type u64",
                ),
                &message,
            )?;
            ensure(!message.contains("badtoken"), &message)?;
            Ok(())
        },
    )
}

#[test]
fn fail_closed_a_non_unicode_limit_variable_is_an_error_without_its_value() -> TestResult {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let value = std::ffi::OsString::from_vec(b"badbytes-\xff".to_vec());
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some(std::ffi::OsString::from("lean"))),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some(value)),
        ],
        || {
            let error = startup_limits_from_env().check_err("non-unicode must fail startup")?;
            let message = error.to_string();
            ensure(
                message.starts_with("SLUGAUDIT_MAX_FILE_BYTES is set but is not valid Unicode"),
                &message,
            )?;
            ensure(!message.contains("badbytes"), &message)?;
            Ok(())
        },
    )
}

#[test]
fn fail_closed_undetectable_memory_without_an_explicit_profile_names_the_variable_to_set()
-> TestResult {
    let error = crate::project::validate_startup_config_with(
        &crate::project::GlobalConfig::default(),
        &crate::model::LimitOverrides::default(),
        crate::model::system_ram::detect_from_for_test,
    )
    .check_err("adaptive default needs memory")?;

    ensure(
        matches!(
            error,
            crate::project::ProjectLimitsError::Env(
                crate::model::EnvConfigError::MemoryUndetectable { .. }
            )
        ),
        format!("{error:?}"),
    )?;
    ensure(
        error
            .to_string()
            .contains("set SLUGAUDIT_PROFILE to `lean` or `audit`"),
        format!("{error}"),
    )
}

#[test]
fn fail_closed_an_explicit_profile_runs_without_detectable_memory() -> TestResult {
    let limits = crate::project::validate_startup_config_with(
        &crate::project::GlobalConfig {
            profile: Some(crate::model::AuditProfile::Lean),
            ..Default::default()
        },
        &crate::model::LimitOverrides::default(),
        crate::model::system_ram::detect_from_for_test,
    )
    .check("lean needs no detection")?;

    ensure_eq(
        &limits.max_file_bytes,
        &(32 * 1024 * 1024),
        "explicit lean profile uses 32 MiB max_file_bytes without host memory detection",
    )
}

#[test]
fn the_default_limits_ignore_the_environment_and_the_host_memory() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let baseline = ResourceLimits::default();

    let under_env = temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("1")),
        ],
        ResourceLimits::default,
    );

    ensure_eq(
        &under_env,
        &baseline,
        "ResourceLimits::default ignores ambient environment variables",
    )?;
    ensure_eq(
        &baseline,
        &crate::model::profile::adaptive_limits(DEFAULT_BASELINE_MEMORY),
        "ResourceLimits::default matches the fixed 16 GiB adaptive baseline",
    )
}
