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
        let limits = limits_for_profile(profile);
        assert!(limits.max_file_bytes > 0);
        assert!(limits.max_total_import_bytes >= limits.max_file_bytes);
        assert!(limits.max_query_response_bytes > 0);
    }
}

#[test]
fn a_set_but_unknown_profile_warns_and_selects_nothing() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

    temp_env::with_var("SLUGAUDIT_PROFILE", Some("audut"), || {
        let (profile, logs) = crate::store::test_capture::capture_warns(profile_from_env);
        assert!(profile.is_none(), "the default profile applies");
        assert!(
            logs.contains("not a known profile"),
            "a discarded operator setting must be visible, or a typo'd \
             profile silently does nothing:\n{logs}"
        );
    });
}

#[test]
fn a_non_unicode_profile_warns_without_logging_its_value() {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let value = std::ffi::OsString::from_vec(b"secret-\xff-profile".to_vec());

    temp_env::with_var("SLUGAUDIT_PROFILE", Some(value), || {
        let (profile, logs) = crate::store::test_capture::capture_warns(profile_from_env);

        assert!(profile.is_none(), "the default profile applies");
        assert!(logs.contains("could not be read"), "{logs}");
        assert!(logs.contains("not valid unicode"), "{logs}");
        assert!(
            !logs.contains("secret-"),
            "the value must not be logged:\n{logs}"
        );
    });
}

#[test]
fn an_unset_profile_selects_nothing_without_warning() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

    temp_env::with_var_unset("SLUGAUDIT_PROFILE", || {
        let (profile, logs) = crate::store::test_capture::capture_warns(|| {
            tracing::warn!("control-line-probe-profile-unset");
            profile_from_env()
        });
        assert!(profile.is_none());
        assert!(
            logs.contains("control-line-probe-profile-unset"),
            "capture liveness control"
        );
        assert!(
            !logs.contains("profile override"),
            "an operator who set nothing must not be warned at:\n{logs}"
        );
    });
}

#[test]
fn a_known_profile_resolves_without_warning() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

    temp_env::with_var("SLUGAUDIT_PROFILE", Some("lean"), || {
        let (profile, logs) = crate::store::test_capture::capture_warns(|| {
            tracing::warn!("control-line-probe-profile-known");
            profile_from_env()
        });
        assert_eq!(profile, Some(AuditProfile::Lean));
        assert!(
            logs.contains("control-line-probe-profile-known"),
            "capture liveness control"
        );
        assert!(
            !logs.contains("profile override"),
            "a valid profile must not warn:\n{logs}"
        );
    });
}

#[test]
fn watch_scope_caps_are_bounded_per_profile() {
    let lean = limits_for_profile(AuditProfile::Lean);
    assert_eq!(lean.max_watch_entries, 50_000);
    assert_eq!(lean.max_watch_depth, 32);

    for profile in [AuditProfile::Audit, AuditProfile::Adaptive] {
        let limits = limits_for_profile(profile);
        assert_eq!(limits.max_watch_entries, 200_000, "{profile:?}");
        assert_eq!(limits.max_watch_depth, 64, "{profile:?}");
    }
}

#[test]
fn adaptive_file_and_query_caps_never_exceed_sqlite_max() {
    let limits = limits_for_profile(AuditProfile::Adaptive);

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
        let limits = limits_for_profile(profile);

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
        let limits = limits_for_profile(profile);

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
        let limits = limits_for_profile(profile);

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
        let limits = limits_for_profile(profile);

        assert_eq!(limits.max_structure_query_bytes, 16 * 1024, "{profile:?}");
    }
}
