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
