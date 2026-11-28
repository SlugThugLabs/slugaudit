use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn parse_str_recognizes_profiles() -> TestResult {
    ensure_eq(
        &AuditProfile::parse_str("lean"),
        &Some(AuditProfile::Lean),
        "lowercase lean parses to AuditProfile::Lean",
    )?;
    ensure_eq(
        &AuditProfile::parse_str("AUDIT"),
        &Some(AuditProfile::Audit),
        "uppercase AUDIT parses case-insensitively to AuditProfile::Audit",
    )?;
    ensure_eq(
        &AuditProfile::parse_str("adaptive"),
        &Some(AuditProfile::Adaptive),
        "lowercase adaptive parses to AuditProfile::Adaptive",
    )?;
    ensure_eq(
        &AuditProfile::parse_str("unknown"),
        &None,
        "unrecognized profile name returns None",
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
        ensure(
            limits.max_file_bytes > 0,
            "profile max_file_bytes is positive",
        )?;
        ensure(
            limits.max_total_import_bytes >= limits.max_file_bytes,
            "profile import budget is at least one max-size file",
        )?;
        ensure(
            limits.max_query_response_bytes > 0,
            "profile query response cap is positive",
        )?;
    }
    Ok(())
}

#[test]
fn profiles_bound_sync_wall_clock_to_30s_lean_and_90s_audit_and_adaptive() -> TestResult {
    ensure_eq(
        &limits_for_profile(AuditProfile::Lean)
            .check("lean limits")?
            .max_sync_wall_clock,
        &Duration::from_secs(30),
        "lean profile defaults to a 30-second sync wall-clock budget",
    )?;
    ensure_eq(
        &limits_for_profile(AuditProfile::Audit)
            .check("audit limits")?
            .max_sync_wall_clock,
        &Duration::from_secs(90),
        "audit profile defaults to a 90-second sync wall-clock budget",
    )?;
    ensure_eq(
        &limits_for_profile(AuditProfile::Adaptive)
            .check("adaptive limits")?
            .max_sync_wall_clock,
        &Duration::from_secs(90),
        "adaptive profile defaults to a 90-second sync wall-clock budget",
    )?;
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
            "unknown profile error names SLUGAUDIT_PROFILE and lists valid options",
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
            "unset SLUGAUDIT_PROFILE returns None",
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
            "SLUGAUDIT_PROFILE=lean resolves to AuditProfile::Lean",
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
        "adaptive max_file_bytes stays within SQLite's maximum value length",
    )?;
    ensure(
        limits.max_query_value_bytes
            <= usize::try_from(SQLITE_MAX_VALUE_BYTES).check("the cap fits usize")?,
        "adaptive max_query_value_bytes stays within SQLite's maximum value length",
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
            "default page size fits within query response ceiling",
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
        "audit result store budget defaults to 1 GiB",
    )?;
    ensure_eq(
        &limits_for_profile(AuditProfile::Lean)
            .check("profile limits")?
            .max_result_store_bytes,
        &(128 * 1024 * 1024),
        "lean result store budget defaults to 128 MiB",
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

#[test]
fn adaptive_limits_scales_safely_for_small_and_large_hosts() -> TestResult {
    const GIB: u64 = 1024 * 1024 * 1024;

    let one_gib = adaptive_limits(SystemMemoryInfo {
        total_bytes: GIB,
        available_bytes: 512 * 1024 * 1024,
    });
    ensure_eq(
        &one_gib.max_file_bytes,
        &(GIB / 10),
        "1 GiB host scales max_file_bytes to 10% of total RAM",
    )?;
    ensure_eq(
        &one_gib.max_total_import_bytes,
        &(512 * 1024 * 1024),
        "1 GiB host scales max_total_import_bytes to half of total RAM",
    )?;
    ensure(
        one_gib.max_parse_bytes < one_gib.max_file_bytes,
        "1 GiB host keeps max_parse_bytes below max_file_bytes",
    )?;
    ensure(
        one_gib.max_total_import_bytes >= one_gib.max_file_bytes,
        "1 GiB host keeps max_total_import_bytes at least max_file_bytes",
    )?;
    ensure_eq(
        &one_gib.max_result_store_bytes,
        &u64::try_from(one_gib.max_query_response_bytes)
            .check("fits u64")?
            .checked_mul(4)
            .check("four responses fit u64")?,
        "1 GiB host keeps max_result_store_bytes at four maximal responses",
    )?;

    let sixteen_gib = adaptive_limits(SystemMemoryInfo {
        total_bytes: 16 * GIB,
        available_bytes: 16 * GIB,
    });
    ensure_eq(
        &sixteen_gib.max_file_bytes,
        &SQLITE_MAX_VALUE_BYTES,
        "16 GiB host clamps max_file_bytes to SQLITE_MAX_VALUE_BYTES",
    )?;
    ensure_eq(
        &sixteen_gib.max_total_import_bytes,
        &(8 * GIB),
        "16 GiB host allocates half of available RAM for imports",
    )?;
    ensure(
        sixteen_gib.max_parse_bytes < sixteen_gib.max_file_bytes,
        "16 GiB host keeps max_parse_bytes below max_file_bytes",
    )?;
    ensure(
        sixteen_gib.max_total_import_bytes >= sixteen_gib.max_file_bytes,
        "16 GiB host keeps max_total_import_bytes at least max_file_bytes",
    )?;
    ensure_eq(
        &sixteen_gib.max_result_store_bytes,
        &u64::try_from(sixteen_gib.max_query_response_bytes)
            .check("fits u64")?
            .checked_mul(4)
            .check("four responses fit u64")?,
        "16 GiB host keeps max_result_store_bytes at four maximal responses",
    )?;

    let zero_ram = adaptive_limits(SystemMemoryInfo {
        total_bytes: 0,
        available_bytes: 0,
    });
    ensure_eq(
        &zero_ram.max_file_bytes,
        &(32 * 1024 * 1024),
        "zero-RAM synthetic input clamps max_file_bytes to 32 MiB",
    )?;
    ensure(
        zero_ram.max_parse_bytes < zero_ram.max_file_bytes,
        "zero-RAM synthetic input keeps max_parse_bytes below max_file_bytes",
    )?;
    ensure(
        zero_ram.max_total_import_bytes >= zero_ram.max_file_bytes,
        "zero-RAM synthetic input keeps max_total_import_bytes at least max_file_bytes",
    )?;
    Ok(())
}
