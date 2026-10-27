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
    assert!(limits.max_watch_entries > 0);
    assert!(limits.max_watch_depth > 0);
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

#[test]
fn watch_scope_limits_default_to_the_adaptive_profile_caps() {
    let limits = ResourceLimits::default();
    assert_eq!(limits.max_watch_entries, 200_000);
    assert_eq!(limits.max_watch_depth, 64);
}

#[test]
fn from_env_overrides_watch_scope_limits() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_WATCH_ENTRIES", Some("1234")),
            ("SLUGAUDIT_MAX_WATCH_DEPTH", Some("7")),
        ],
        || {
            let limits = ResourceLimits::from_env();
            assert_eq!(limits.max_watch_entries, 1_234);
            assert_eq!(limits.max_watch_depth, 7);
            // Unset neighbours remain at the selected profile's defaults.
            assert_eq!(limits.max_file_bytes, 32 * 1024 * 1024);
        },
    );
}

#[test]
fn an_override_above_sqlite_max_is_clamped() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("2000000000")),
            ("SLUGAUDIT_MAX_QUERY_VALUE_BYTES", Some("2000000000")),
        ],
        || {
            let limits = ResourceLimits::from_env();
            assert_eq!(limits.max_file_bytes, SQLITE_MAX_VALUE_BYTES);
            assert_eq!(
                limits.max_query_value_bytes,
                SQLITE_MAX_VALUE_BYTES as usize
            );
        },
    );
}

#[test]
fn the_default_page_size_is_64_kib_and_the_env_var_overrides_it() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var_unset("SLUGAUDIT_DEFAULT_PAGE_BYTES", || {
        assert_eq!(ResourceLimits::from_env().default_page_bytes, 65_536);
    });

    temp_env::with_var("SLUGAUDIT_DEFAULT_PAGE_BYTES", Some("1234"), || {
        assert_eq!(ResourceLimits::from_env().default_page_bytes, 1_234);
    });
}

#[test]
fn the_max_parse_bytes_env_var_overrides_the_profile_default() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("65536"), || {
        let limits = ResourceLimits::from_env();

        assert_eq!(limits.max_parse_bytes, 65_536);
    });
}

#[test]
fn an_unparseable_max_parse_bytes_keeps_the_default_and_says_so() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("two-mib"), || {
        let (limits, logs) = crate::store::test_capture::capture_warns(ResourceLimits::from_env);

        assert_eq!(limits.max_parse_bytes, 2 * 1024 * 1024);
        assert!(
            logs.contains("SLUGAUDIT_MAX_PARSE_BYTES")
                && logs.contains("not a valid value and was ignored"),
            "{logs}"
        );
    });
}

#[test]
fn a_max_parse_bytes_env_var_above_max_file_bytes_is_clamped_and_reported() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("1048576")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("4194304")),
        ],
        || {
            let (limits, logs) =
                crate::store::test_capture::capture_warns(ResourceLimits::from_env);

            assert_eq!(limits.max_parse_bytes, 1_048_576);
            assert!(
                logs.contains("max_parse_bytes exceeds max_file_bytes; clamping"),
                "{logs}"
            );
        },
    );
}

#[test]
fn the_max_parse_wall_clock_ms_env_var_overrides_the_profile_default() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS", Some("750"), || {
        let limits = ResourceLimits::from_env();

        assert_eq!(limits.max_parse_wall_clock_ms, 750);
    });
}

#[test]
fn an_unparseable_max_parse_wall_clock_ms_keeps_the_default_and_says_so() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS", Some("soon"), || {
        let (limits, logs) = crate::store::test_capture::capture_warns(ResourceLimits::from_env);

        assert_eq!(limits.max_parse_wall_clock_ms, 5_000);
        assert!(
            logs.contains("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS")
                && logs.contains("not a valid value and was ignored"),
            "{logs}"
        );
    });
}

#[test]
fn the_max_structure_query_bytes_env_var_overrides_the_16_kib_default() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    assert_eq!(ResourceLimits::default().max_structure_query_bytes, 16_384);
    temp_env::with_var("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES", Some("300"), || {
        assert_eq!(ResourceLimits::from_env().max_structure_query_bytes, 300);
    });
}

#[test]
fn the_result_store_env_vars_override_the_profile_defaults() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_RESULT_STORE_BYTES", Some("123456")),
            ("SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS", Some("42")),
        ],
        || {
            let limits = ResourceLimits::from_env();

            assert_eq!(limits.max_result_store_bytes, 123_456);
            assert_eq!(limits.result_active_grace_secs, 42);
        },
    );
}
