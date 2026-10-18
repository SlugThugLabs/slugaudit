use super::*;

#[test]
fn defaults_are_strictly_positive() {
    let limits = ResourceLimits::default();
    assert!(limits.max_file_bytes > 0);
    assert!(limits.max_total_import_bytes >= limits.max_file_bytes);
    assert!(limits.max_query_response_bytes > 0);
    assert!(limits.max_query_vm_steps > 0);
    assert!(limits.max_query_wall_clock.as_millis() > 0);
    assert!(limits.max_query_value_bytes > 0);
    assert!(limits.max_query_value_bytes <= limits.max_query_response_bytes);
    assert!(limits.max_structure_query_bytes > 0);
    assert!(limits.max_structure_matches > 0);
    assert!(limits.max_structure_execution_time.as_millis() > 0);
    assert!(limits.max_sync_wall_clock.as_millis() > 0);
}

#[test]
fn from_env_uses_defaults_when_no_vars_are_set() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var_unset("SLUGAUDIT_MAX_FILE_BYTES", || {
        let limits = ResourceLimits::from_env();
        assert_eq!(
            limits.max_file_bytes,
            ResourceLimits::default().max_file_bytes
        );
    });
}

#[test]
fn from_env_overrides_u64_fields() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("16777216")),
        ],
        || {
            let limits = ResourceLimits::from_env();
            assert_eq!(limits.max_file_bytes, 16_777_216);
            // Other fields remain at the selected profile's defaults.
            assert_eq!(limits.max_total_import_bytes, 2 * 1024 * 1024 * 1024);
        },
    );
}

#[test]
fn from_env_overrides_duration_fields() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS", Some("120"), || {
        let limits = ResourceLimits::from_env();
        assert_eq!(limits.max_sync_wall_clock, Duration::from_secs(120));
    });
}

#[test]
fn an_unparseable_override_falls_back_to_the_default_and_says_so() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_FILE_BYTES", Some("not-a-number"), || {
        let (limits, logs) = crate::store::test_capture::capture_warns(ResourceLimits::from_env);
        assert_eq!(
            limits.max_file_bytes,
            ResourceLimits::default().max_file_bytes,
            "the default still applies"
        );
        assert!(
            logs.contains("not a valid value and was ignored"),
            "the operator's discarded setting must be visible, or an intended \
             limit silently does nothing:\n{logs}"
        );
    });
}

#[test]
fn an_unparseable_override_warning_carries_the_parse_error() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_FILE_BYTES", Some("not-a-number"), || {
        let (_, logs) = crate::store::test_capture::capture_warns(ResourceLimits::from_env);

        assert!(
            logs.contains("invalid digit found in string"),
            "the source parse error must be logged:\n{logs}"
        );
    });
}

#[test]
fn a_non_unicode_override_warns_without_logging_its_value() {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let value = std::ffi::OsString::from_vec(b"secret-\xff-value".to_vec());
    temp_env::with_var("SLUGAUDIT_MAX_FILE_BYTES", Some(value), || {
        let (limits, logs) = crate::store::test_capture::capture_warns(ResourceLimits::from_env);

        assert_eq!(
            limits.max_file_bytes,
            ResourceLimits::default().max_file_bytes
        );
        assert!(logs.contains("could not read a limit override"), "{logs}");
        assert!(logs.contains("not valid unicode"), "{logs}");
        assert!(
            !logs.contains("secret-"),
            "the value must not be logged:\n{logs}"
        );
    });
}

#[test]
fn multiple_overrides_compose() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("1048576")),
            ("SLUGAUDIT_MAX_QUERY_VM_STEPS", Some("5000000")),
            ("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS", Some("30")),
        ],
        || {
            let limits = ResourceLimits::from_env();
            assert_eq!(limits.max_file_bytes, 1_048_576);
            assert_eq!(limits.max_query_vm_steps, 5_000_000);
            assert_eq!(limits.max_sync_wall_clock, Duration::from_secs(30));
            // Unset fields stay at defaults
            assert_eq!(
                limits.max_query_response_bytes,
                ResourceLimits::default().max_query_response_bytes
            );
        },
    );
}
