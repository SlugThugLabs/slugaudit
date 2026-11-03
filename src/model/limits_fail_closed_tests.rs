//! Fail-closed environment handling and the process-wide limits cell.

use super::*;

#[test]
fn fail_closed_an_invalid_limit_variable_is_an_error_naming_it_not_its_value() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("12MBsecret")),
        ],
        || {
            let error = ResourceLimits::from_env().check_err("a typo'd limit must fail startup")?;

            let message = error.to_string();
            ensure(message.starts_with(
                    "SLUGAUDIT_MAX_FILE_BYTES is set but is not valid; expected a value of type u64"
                ), &message)?;
            ensure(!message.contains("secret"), &message)?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn fail_closed_a_non_unicode_limit_variable_is_an_error_without_its_value() -> TestResult {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let value = std::ffi::OsString::from_vec(b"secret-\xff".to_vec());
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some(std::ffi::OsString::from("lean"))),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some(value)),
        ],
        || {
            let error = ResourceLimits::from_env().check_err("non-unicode must fail startup")?;

            let message = error.to_string();
            ensure(
                message.starts_with("SLUGAUDIT_MAX_FILE_BYTES is set but is not valid Unicode"),
                &message,
            )?;
            ensure(!message.contains("secret"), &message)?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn fail_closed_undetectable_memory_without_an_explicit_profile_names_the_variable_to_set()
-> TestResult {
    let error = base_limits(
        crate::model::AuditProfile::default(),
        crate::model::system_ram::detect_from_for_test,
    )
    .check_err("adaptive default needs memory")?;

    ensure(
        matches!(error, EnvConfigError::MemoryUndetectable { .. }),
        format!("{error}"),
    )?;
    ensure(
        error
            .to_string()
            .contains("set SLUGAUDIT_PROFILE to `lean` or `audit`"),
        format!("{error}"),
    )?;
    Ok(())
}

#[test]
fn fail_closed_an_explicit_profile_runs_without_detectable_memory() -> TestResult {
    let limits = base_limits(
        crate::model::AuditProfile::Lean,
        crate::model::system_ram::detect_from_for_test,
    )
    .check("lean needs no detection")?;

    ensure_eq(
        &limits.max_file_bytes,
        &(32 * 1024 * 1024),
        "limits.max_file_bytes == 32 * 1024 * 1024",
    )?;
    Ok(())
}

#[test]
fn fail_closed_an_uninstalled_limits_cell_with_an_invalid_env_returns_the_error_and_installs_nothing()
-> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK
        .lock()
        .check("take the env lock")?;
    let cell = OnceLock::new();
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("0")),
        ],
        || {
            let error = install_limits(&cell).check_err("a zero limit must not install")?;

            ensure(
                matches!(&error, EnvConfigError::Zero { variable } if variable == "SLUGAUDIT_MAX_PARSE_BYTES"),
                format!("{error:?}"),
            )?;
            ensure(cell.get().is_none(), "an invalid env must install nothing")?;
            Ok(())
        },
    )
}

#[test]
fn an_uninstalled_limits_cell_with_a_valid_env_installs_once_and_keeps_the_first_value()
-> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let cell = OnceLock::new();
    let first = temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("4096")),
        ],
        || install_limits(&cell).map(|limits| limits.max_parse_bytes),
    )
    .check("valid env installs")?;
    ensure_eq(&first, &4096, "first == 4096")?;

    // The environment is not read again once the limits are installed.
    let second = temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("0"), || {
        install_limits(&cell).map(|limits| limits.max_parse_bytes)
    })
    .check("installed limits are returned without re-reading the env")?;
    ensure_eq(&second, &4096, "second == 4096")?;
    Ok(())
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

    ensure_eq(&under_env, &baseline, "under_env == baseline")?;
    ensure_eq(
        &baseline,
        &crate::model::profile::adaptive_limits(DEFAULT_BASELINE_MEMORY),
        "baseline == crate::model::profile::adaptive_limits(DEFAULT_BASELINE_MEMORY)",
    )?;
    Ok(())
}
