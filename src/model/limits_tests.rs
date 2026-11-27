use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn startup_limits_from_env() -> Result<ResourceLimits, crate::project::ProjectLimitsError> {
    let _cfg = crate::project::GlobalConfigGuardForTest::none();
    crate::project::validate_startup_config()
}

#[test]
fn defaults_are_strictly_positive() -> TestResult {
    let limits = ResourceLimits::default();
    ensure(
        limits.max_file_bytes > 0,
        "default max_file_bytes is positive",
    )?;
    ensure(
        limits.max_total_import_bytes >= limits.max_file_bytes,
        "default import budget is at least one max-size file",
    )?;
    ensure(
        limits.max_query_response_bytes > 0,
        "default query response ceiling is positive",
    )?;
    ensure(
        limits.max_query_vm_steps > 0,
        "default query VM step budget is positive",
    )?;
    ensure(
        limits.max_query_wall_clock.as_millis() > 0,
        "default query wall-clock budget is positive",
    )?;
    ensure(
        limits.max_query_value_bytes > 0,
        "default query value byte cap is positive",
    )?;
    ensure(
        limits.max_query_value_bytes <= limits.max_query_response_bytes,
        "default query value cap fits within query response ceiling",
    )?;
    ensure(
        limits.max_structure_query_bytes > 0,
        "default structure query byte cap is positive",
    )?;
    ensure(
        limits.max_structure_matches > 0,
        "default structure match ceiling is positive",
    )?;
    ensure(
        limits.max_structure_execution_time.as_millis() > 0,
        "default structure execution budget is positive",
    )?;
    ensure(
        limits.max_sync_wall_clock.as_millis() > 0,
        "default sync wall-clock budget is positive",
    )?;
    Ok(())
}

#[test]
fn from_env_uses_defaults_when_no_vars_are_set() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars_unset(["SLUGAUDIT_PROFILE", "SLUGAUDIT_MAX_FILE_BYTES"], || {
        let limits = startup_limits_from_env().check("valid env")?;
        let host = crate::model::detect_system_memory().check("this host reports its memory")?;
        ensure_eq(
            &limits.max_file_bytes,
            &crate::model::profile::adaptive_limits(host).max_file_bytes,
            "unset environment variables resolve to the adaptive host baseline",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn from_env_overrides_u64_fields() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("16777216")),
        ],
        || {
            let limits = startup_limits_from_env().check("valid env")?;
            ensure_eq(
                &limits.max_file_bytes,
                &16_777_216,
                "SLUGAUDIT_MAX_FILE_BYTES overrides max_file_bytes",
            )?;
            // Other fields remain at the selected profile's defaults.
            ensure_eq(
                &limits.max_total_import_bytes,
                &(2 * 1024 * 1024 * 1024),
                "unmentioned fields stay at the selected profile's default",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn from_env_overrides_duration_fields() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS", Some("120"), || {
        let limits = startup_limits_from_env().check("valid env")?;
        ensure_eq(
            &limits.max_sync_wall_clock,
            &Duration::from_mins(2),
            "SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS overrides sync wall-clock duration",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn fail_closed_an_unparseable_override_error_carries_the_parse_cause() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("not-a-number")),
        ],
        || {
            let error = startup_limits_from_env().check_err("must fail startup")?;

            ensure(
                error.to_string().contains("invalid digit found in string"),
                format!("{error}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn multiple_overrides_compose() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("1048576")),
            ("SLUGAUDIT_MAX_QUERY_VM_STEPS", Some("5000000")),
            ("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS", Some("45")),
        ],
        || {
            let limits = startup_limits_from_env().check("valid env")?;
            ensure_eq(
                &limits.max_file_bytes,
                &1_048_576,
                "max_file_bytes reflects environment override",
            )?;
            ensure_eq(
                &limits.max_query_vm_steps,
                &5_000_000,
                "max_query_vm_steps reflects environment override",
            )?;
            ensure_eq(
                &limits.max_sync_wall_clock,
                &Duration::from_secs(45),
                "max_sync_wall_clock reflects environment override",
            )?;
            let baseline = crate::model::limits_for_profile(crate::model::AuditProfile::default())
                .check("host memory")?;
            ensure_eq(
                &limits.max_query_response_bytes,
                &baseline.max_query_response_bytes,
                "unset fields preserve the adaptive host baseline",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn an_override_above_sqlite_max_is_clamped() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("2000000000")),
            ("SLUGAUDIT_MAX_QUERY_VALUE_BYTES", Some("2000000000")),
        ],
        || {
            let limits = startup_limits_from_env().check("valid env")?;
            ensure_eq(
                &limits.max_file_bytes,
                &SQLITE_MAX_VALUE_BYTES,
                "oversized SLUGAUDIT_MAX_FILE_BYTES clamps to SQLITE_MAX_VALUE_BYTES",
            )?;
            ensure_eq(
                &limits.max_query_value_bytes,
                &usize::try_from(SQLITE_MAX_VALUE_BYTES).check("the cap fits usize")?,
                "oversized SLUGAUDIT_MAX_QUERY_VALUE_BYTES clamps to SQLITE_MAX_VALUE_BYTES",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn the_default_page_size_is_64_kib_and_the_env_var_overrides_it() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var_unset("SLUGAUDIT_DEFAULT_PAGE_BYTES", || {
        ensure_eq(
            &startup_limits_from_env()
                .check("valid env")?
                .default_page_bytes,
            &65_536,
            "default_page_bytes defaults to 64 KiB when unset",
        )?;
        Ok(())
    })?;

    temp_env::with_var("SLUGAUDIT_DEFAULT_PAGE_BYTES", Some("1234"), || {
        ensure_eq(
            &startup_limits_from_env()
                .check("valid env")?
                .default_page_bytes,
            &1_234,
            "SLUGAUDIT_DEFAULT_PAGE_BYTES overrides default_page_bytes",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn the_max_parse_bytes_env_var_overrides_the_profile_default() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("65536"), || {
        let limits = startup_limits_from_env().check("valid env")?;

        ensure_eq(
            &limits.max_parse_bytes,
            &65_536,
            "SLUGAUDIT_MAX_PARSE_BYTES overrides max_parse_bytes",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn fail_closed_an_unparseable_max_parse_bytes_is_an_error_naming_it() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("two-mib")),
        ],
        || {
            let error = startup_limits_from_env().check_err("must fail startup")?;

            ensure(
                error
                    .to_string()
                    .starts_with("SLUGAUDIT_MAX_PARSE_BYTES is set but is not valid"),
                format!("{error}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn a_max_parse_bytes_env_var_above_max_file_bytes_is_clamped_and_reported() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("1048576")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("4194304")),
        ],
        || {
            let (result, logs) = crate::util::test_capture::capture_warns(startup_limits_from_env)?;
            let limits = result.check("valid env")?;

            ensure_eq(
                &limits.max_parse_bytes,
                &1_048_576,
                "max_parse_bytes clamps down to max_file_bytes",
            )?;
            ensure(
                logs.contains("max_parse_bytes exceeds max_file_bytes; clamping"),
                &logs,
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn the_max_parse_wall_clock_ms_env_var_overrides_the_profile_default() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS", Some("750"), || {
        let limits = startup_limits_from_env().check("valid env")?;

        ensure_eq(
            &limits.max_parse_wall_clock_ms,
            &750,
            "SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS overrides max_parse_wall_clock_ms",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn fail_closed_an_unparseable_max_parse_wall_clock_ms_is_an_error_naming_it() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS", Some("fast")),
        ],
        || {
            let error = startup_limits_from_env().check_err("must fail startup")?;

            ensure(
                error
                    .to_string()
                    .starts_with("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS is set but is not valid"),
                format!("{error}"),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn the_max_structure_query_bytes_env_var_overrides_the_16_kib_default() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    ensure_eq(
        &ResourceLimits::default().max_structure_query_bytes,
        &16_384,
        "default max_structure_query_bytes is 16 KiB",
    )?;
    temp_env::with_var("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES", Some("300"), || {
        ensure_eq(
            &startup_limits_from_env()
                .check("valid env")?
                .max_structure_query_bytes,
            &300,
            "SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES overrides max_structure_query_bytes",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn the_result_store_env_vars_override_the_profile_defaults() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_RESULT_STORE_BYTES", Some("123456")),
            ("SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS", Some("42")),
        ],
        || {
            let limits = startup_limits_from_env().check("valid env")?;

            ensure_eq(
                &limits.max_result_store_bytes,
                &123_456,
                "SLUGAUDIT_MAX_RESULT_STORE_BYTES overrides max_result_store_bytes",
            )?;
            ensure_eq(
                &limits.result_active_grace_secs,
                &42,
                "SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS overrides result_active_grace_secs",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn result_store_surface_budget_follows_the_overridden_response_ceiling() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES", Some("1000")),
            ("SLUGAUDIT_MAX_RESULT_STORE_BYTES", None),
        ],
        || {
            let limits = startup_limits_from_env().check("valid env")?;

            ensure_eq(
                &limits.max_query_response_bytes,
                &1000,
                "SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES sets response ceiling",
            )?;
            ensure_eq(
                &limits.max_result_store_bytes,
                &4000,
                "unpinned result store budget scales to four times the response ceiling",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn result_store_surface_own_budget_variable_beats_the_response_ceiling() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES", Some("1000")),
            ("SLUGAUDIT_MAX_RESULT_STORE_BYTES", Some("777")),
        ],
        || {
            ensure_eq(
                &startup_limits_from_env()
                    .check("valid env")?
                    .max_result_store_bytes,
                &777,
                "explicit SLUGAUDIT_MAX_RESULT_STORE_BYTES overrides derived budget",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[path = "limits_fail_closed_tests.rs"]
mod fail_closed_tests;
