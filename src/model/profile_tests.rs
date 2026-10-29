use super::*;

#[test]
fn parse_str_recognizes_profiles() {
    assert_eq!(AuditProfile::parse_str("lean"), Some(AuditProfile::Lean));
    assert_eq!(AuditProfile::parse_str("AUDIT"), Some(AuditProfile::Audit));
    assert_eq!(
        AuditProfile::parse_str("adaptive"),
        Some(AuditProfile::Adaptive)
    );
    assert_eq!(AuditProfile::parse_str("unknown"), None);
}

#[test]
fn profiles_provide_positive_limits() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");
        assert!(limits.max_file_bytes > 0);
        assert!(limits.max_total_import_bytes >= limits.max_file_bytes);
        assert!(limits.max_query_response_bytes > 0);
    }
}

#[test]
fn fail_closed_a_set_but_unknown_profile_is_an_error_naming_the_variable_not_the_value() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

    temp_env::with_var("SLUGAUDIT_PROFILE", Some("audut"), || {
        let error = profile_from_env().expect_err("a typo'd profile must fail startup");

        assert_eq!(
            error.to_string(),
            "SLUGAUDIT_PROFILE is set but is not valid; expected one of `lean`, `audit`, `adaptive`"
        );
    });
}

#[test]
fn fail_closed_a_non_unicode_profile_is_an_error_without_its_value() {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let value = std::ffi::OsString::from_vec(b"secret-\xff-profile".to_vec());

    temp_env::with_var("SLUGAUDIT_PROFILE", Some(value), || {
        let error = profile_from_env().expect_err("a non-unicode profile must fail startup");

        let message = error.to_string();
        assert!(
            message.starts_with("SLUGAUDIT_PROFILE is set but is not valid Unicode"),
            "{message}"
        );
        assert!(!message.contains("secret-"), "{message}");
    });
}

#[test]
fn an_unset_profile_selects_nothing() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

    temp_env::with_var_unset("SLUGAUDIT_PROFILE", || {
        assert_eq!(profile_from_env().expect("unset is valid"), None);
    });
}

#[test]
fn a_known_profile_resolves() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

    temp_env::with_var("SLUGAUDIT_PROFILE", Some("lean"), || {
        assert_eq!(profile_from_env().expect("valid"), Some(AuditProfile::Lean));
    });
}

fn undetectable() -> Result<SystemMemoryInfo, MemoryDetectError> {
    super::super::system_ram::detect_from_for_test()
}

#[test]
fn fail_closed_adaptive_without_detectable_memory_is_an_error() {
    let error = limits_for_profile_with(AuditProfile::Adaptive, undetectable)
        .expect_err("adaptive must not be sized off a guessed machine");

    assert!(
        error
            .to_string()
            .starts_with("system memory could not be detected"),
        "{error}"
    );
}

#[test]
fn fail_closed_explicit_lean_and_audit_need_no_memory_detection() {
    for profile in [AuditProfile::Lean, AuditProfile::Audit] {
        limits_for_profile_with(profile, undetectable).expect("no detection needed");
    }
}

#[test]
fn watch_scope_caps_are_bounded_per_profile() {
    let lean = limits_for_profile(AuditProfile::Lean).expect("profile limits");
    assert_eq!(lean.max_watch_entries, 50_000);
    assert_eq!(lean.max_watch_depth, 32);

    for profile in [AuditProfile::Audit, AuditProfile::Adaptive] {
        let limits = limits_for_profile(profile).expect("profile limits");
        assert_eq!(limits.max_watch_entries, 200_000, "{profile:?}");
        assert_eq!(limits.max_watch_depth, 64, "{profile:?}");
    }
}

#[test]
fn adaptive_file_and_query_caps_never_exceed_sqlite_max() {
    let limits = limits_for_profile(AuditProfile::Adaptive).expect("profile limits");

    assert!(limits.max_file_bytes <= SQLITE_MAX_VALUE_BYTES);
    assert!(limits.max_query_value_bytes <= SQLITE_MAX_VALUE_BYTES as usize);
}

#[test]
fn every_profile_defaults_to_a_64_kib_page_under_its_ceiling() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");

        assert_eq!(limits.default_page_bytes, 65_536, "{profile:?}");
        assert!(limits.default_page_bytes <= limits.max_query_response_bytes);
    }
}

#[test]
fn every_profile_parses_at_most_2_mib_per_file_below_its_file_cap() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");

        assert_eq!(limits.max_parse_bytes, 2 * 1024 * 1024, "{profile:?}");
        assert!(
            limits.max_parse_bytes < limits.max_file_bytes,
            "{profile:?}"
        );
    }
}

#[test]
fn every_profile_gives_one_file_5_seconds_of_parse_time_below_the_sync_budget() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");

        assert_eq!(limits.max_parse_wall_clock_ms, 5_000, "{profile:?}");
        assert!(
            std::time::Duration::from_millis(limits.max_parse_wall_clock_ms)
                < limits.max_sync_wall_clock,
            "{profile:?}"
        );
    }
}

#[test]
fn every_profile_caps_a_structure_query_at_16_kib() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");

        assert_eq!(limits.max_structure_query_bytes, 16 * 1024, "{profile:?}");
    }
}

#[test]
fn every_profile_budgets_the_result_store_at_four_maximal_responses() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");

        assert_eq!(
            limits.max_result_store_bytes,
            4 * limits.max_query_response_bytes as u64,
            "{profile:?}"
        );
    }
    assert_eq!(
        limits_for_profile(AuditProfile::Audit)
            .expect("profile limits")
            .max_result_store_bytes,
        1024 * 1024 * 1024
    );
    assert_eq!(
        limits_for_profile(AuditProfile::Lean)
            .expect("profile limits")
            .max_result_store_bytes,
        128 * 1024 * 1024
    );
}

#[test]
fn every_profile_protects_an_unfinished_result_for_900_seconds() {
    for profile in [
        AuditProfile::Lean,
        AuditProfile::Audit,
        AuditProfile::Adaptive,
    ] {
        let limits = limits_for_profile(profile).expect("profile limits");

        assert_eq!(limits.result_active_grace_secs, 900, "{profile:?}");
    }
}
