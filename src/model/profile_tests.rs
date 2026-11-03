use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn parse_str_recognizes_profiles() -> TestResult {
    ensure_eq(
        &AuditProfile::parse_str("lean"),
        &Some(AuditProfile::Lean),
        "AuditProfile::parse_str(\"lean\") == Some(AuditProfile::Lean)",
    )?;
    ensure_eq(
        &AuditProfile::parse_str("AUDIT"),
        &Some(AuditProfile::Audit),
        "AuditProfile::parse_str(\"AUDIT\") == Some(AuditProfile::Audit)",
    )?;
    ensure_eq(
        &AuditProfile::parse_str("adaptive"),
        &Some(AuditProfile::Adaptive),
        "AuditProfile::parse_str(\"adaptive\") == Some(AuditProfile::Adaptive)",
    )?;
    ensure_eq(
        &AuditProfile::parse_str("unknown"),
        &None,
        "AuditProfile::parse_str(\"unknown\") == None",
    )?;
    Ok(())
}

#[test]
fn profiles_provide_positive_limits() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;
        ensure(limits.max_file_bytes > 0, "limits.max_file_bytes > 0")?;
        ensure(
            limits.max_total_import_bytes >= limits.max_file_bytes,
            "limits.max_total_import_bytes >= limits.max_file_bytes",
        )?;
        ensure(
            limits.max_query_response_bytes > 0,
            "limits.max_query_response_bytes > 0",
        )?;
    }
    Ok(())
}

#[test]
fn fail_closed_a_set_but_unknown_profile_is_an_error_naming_the_variable_not_the_value()
-> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;

    temp_env::with_var("SLUGAUDIT_PROFILE", Some("audut"), || {
        let error = profile_from_env().check_err("a typo'd profile must fail startup")?;

        ensure_eq(
            &error.to_string(),
            &"SLUGAUDIT_PROFILE is set but is not valid; expected one of `lean`, `audit`, `adaptive`",
            "error.to_string() == \"SLUGAUDIT_PROFILE is set but is not valid; expected one of `lean`, `audit`, `adaptive`\"",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn fail_closed_a_non_unicode_profile_is_an_error_without_its_value() -> TestResult {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let value = std::ffi::OsString::from_vec(b"secret-\xff-profile".to_vec());

    temp_env::with_var("SLUGAUDIT_PROFILE", Some(value), || {
        let error = profile_from_env().check_err("a non-unicode profile must fail startup")?;

        let message = error.to_string();
        ensure(
            message.starts_with("SLUGAUDIT_PROFILE is set but is not valid Unicode"),
            &message,
        )?;
        ensure(!message.contains("secret-"), &message)?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn an_unset_profile_selects_nothing() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;

    temp_env::with_var_unset("SLUGAUDIT_PROFILE", || {
        ensure_eq(
            &profile_from_env().check("unset is valid")?,
            &None,
            "profile_from_env().check(\"unset is valid\")? == None",
        )?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn a_known_profile_resolves() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;

    temp_env::with_var("SLUGAUDIT_PROFILE", Some("lean"), || {
        ensure_eq(
            &profile_from_env().check("valid")?,
            &Some(AuditProfile::Lean),
            "profile_from_env().check(\"valid\")? == Some(AuditProfile::Lean)",
        )?;
        Ok(())
    })?;
    Ok(())
}

fn undetectable() -> Result<SystemMemoryInfo, MemoryDetectError> {
    super::super::system_ram::detect_from_for_test()
}

#[test]
fn fail_closed_adaptive_without_detectable_memory_is_an_error() -> TestResult {
    let error = limits_for_profile_with(AuditProfile::Adaptive, undetectable)
        .check_err("adaptive must not be sized off a guessed machine")?;

    ensure(
        error
            .to_string()
            .starts_with("system memory could not be detected"),
        format!("{error}"),
    )?;
    Ok(())
}

#[test]
fn fail_closed_explicit_lean_and_audit_need_no_memory_detection() -> TestResult {
    for profile in [AuditProfile::Lean, AuditProfile::Audit] {
        limits_for_profile_with(profile, undetectable).check("no detection needed")?;
    }
    Ok(())
}

#[test]
fn adaptive_file_and_query_caps_never_exceed_sqlite_max() -> TestResult {
    let limits = limits_for_profile(AuditProfile::Adaptive).check("profile limits")?;

    ensure(
        limits.max_file_bytes <= SQLITE_MAX_VALUE_BYTES,
        "limits.max_file_bytes <= SQLITE_MAX_VALUE_BYTES",
    )?;
    ensure(
        limits.max_query_value_bytes
            <= usize::try_from(SQLITE_MAX_VALUE_BYTES).check("the cap fits usize")?,
        "limits.max_query_value_bytes <= usize::try_from(SQLITE_MAX_VALUE_BYTES).check(\"the cap fits usize\")?",
    )?;
    Ok(())
}

#[test]
fn every_profile_defaults_to_a_64_kib_page_under_its_ceiling() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;

        ensure_eq(&limits.default_page_bytes, &65_536, format!("{profile:?}"))?;
        ensure(
            limits.default_page_bytes <= limits.max_query_response_bytes,
            "limits.default_page_bytes <= limits.max_query_response_bytes",
        )?;
    }
    Ok(())
}

#[test]
fn every_profile_parses_at_most_2_mib_per_file_below_its_file_cap() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;

        ensure_eq(
            &limits.max_parse_bytes,
            &(2 * 1024 * 1024),
            format!("{profile:?}"),
        )?;
        ensure(
            limits.max_parse_bytes < limits.max_file_bytes,
            format!("{profile:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn every_profile_gives_one_file_5_seconds_of_parse_time_below_the_sync_budget() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;

        ensure_eq(
            &limits.max_parse_wall_clock_ms,
            &5_000,
            format!("{profile:?}"),
        )?;
        ensure(
            std::time::Duration::from_millis(limits.max_parse_wall_clock_ms)
                < limits.max_sync_wall_clock,
            format!("{profile:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn every_profile_caps_a_structure_query_at_16_kib() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;

        ensure_eq(
            &limits.max_structure_query_bytes,
            &(16 * 1024),
            format!("{profile:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn every_profile_budgets_the_result_store_at_four_maximal_responses() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;

        ensure_eq(
            &limits.max_result_store_bytes,
            &(4 * limits.max_query_response_bytes as u64),
            format!("{profile:?}"),
        )?;
    }
    ensure_eq(
        &limits_for_profile(AuditProfile::Audit)
            .check("profile limits")?
            .max_result_store_bytes,
        &(1024 * 1024 * 1024),
        "limits_for_profile(AuditProfile::Audit) .check(\"profile limits\")? .max_result_store_bytes == 1024 * 1024 * 1024",
    )?;
    ensure_eq(
        &limits_for_profile(AuditProfile::Lean)
            .check("profile limits")?
            .max_result_store_bytes,
        &(128 * 1024 * 1024),
        "limits_for_profile(AuditProfile::Lean) .check(\"profile limits\")? .max_result_store_bytes == 128 * 1024 * 1024",
    )?;
    Ok(())
}

#[test]
fn every_profile_protects_an_unfinished_result_for_900_seconds() -> TestResult {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).check("profile limits")?;

        ensure_eq(
            &limits.result_active_grace_secs,
            &900,
            format!("{profile:?}"),
        )?;
    }
    Ok(())
}
