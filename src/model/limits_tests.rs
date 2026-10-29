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
        let limits = ResourceLimits::from_env().expect("valid env");
        // Unset variables leave the Adaptive profile for this host, which is
        // the fixed 16 GiB `Default` only on a host of that size.
        let host = crate::model::detect_system_memory().expect("this host reports its memory");
        assert_eq!(
            limits.max_file_bytes,
            crate::model::profile::adaptive_limits(host).max_file_bytes
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
            let limits = ResourceLimits::from_env().expect("valid env");
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
        let limits = ResourceLimits::from_env().expect("valid env");
        assert_eq!(limits.max_sync_wall_clock, Duration::from_secs(120));
    });
}

#[test]
fn fail_closed_an_unparseable_override_error_carries_the_parse_cause() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("not-a-number")),
        ],
        || {
            let error = ResourceLimits::from_env().expect_err("must fail startup");

            assert!(
                error.to_string().contains("invalid digit found in string"),
                "{error}"
            );
        },
    );
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
            let limits = ResourceLimits::from_env().expect("valid env");
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
            let limits = ResourceLimits::from_env().expect("valid env");
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
            let limits = ResourceLimits::from_env().expect("valid env");
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
        assert_eq!(
            ResourceLimits::from_env()
                .expect("valid env")
                .default_page_bytes,
            65_536
        );
    });

    temp_env::with_var("SLUGAUDIT_DEFAULT_PAGE_BYTES", Some("1234"), || {
        assert_eq!(
            ResourceLimits::from_env()
                .expect("valid env")
                .default_page_bytes,
            1_234
        );
    });
}

#[test]
fn the_max_parse_bytes_env_var_overrides_the_profile_default() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("65536"), || {
        let limits = ResourceLimits::from_env().expect("valid env");

        assert_eq!(limits.max_parse_bytes, 65_536);
    });
}

#[test]
fn fail_closed_an_unparseable_max_parse_bytes_is_an_error_naming_it() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("two-mib")),
        ],
        || {
            let error = ResourceLimits::from_env().expect_err("must fail startup");

            assert!(
                error
                    .to_string()
                    .starts_with("SLUGAUDIT_MAX_PARSE_BYTES is set but is not valid"),
                "{error}"
            );
        },
    );
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
            let (limits, logs) = crate::store::test_capture::capture_warns(|| {
                ResourceLimits::from_env().expect("valid env")
            });

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
        let limits = ResourceLimits::from_env().expect("valid env");

        assert_eq!(limits.max_parse_wall_clock_ms, 750);
    });
}

#[test]
fn fail_closed_an_unparseable_max_parse_wall_clock_ms_is_an_error_naming_it() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS", Some("fast")),
        ],
        || {
            let error = ResourceLimits::from_env().expect_err("must fail startup");

            assert!(
                error
                    .to_string()
                    .starts_with("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS is set but is not valid"),
                "{error}"
            );
        },
    );
}

#[test]
fn the_max_structure_query_bytes_env_var_overrides_the_16_kib_default() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    assert_eq!(ResourceLimits::default().max_structure_query_bytes, 16_384);
    temp_env::with_var("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES", Some("300"), || {
        assert_eq!(
            ResourceLimits::from_env()
                .expect("valid env")
                .max_structure_query_bytes,
            300
        );
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
            let limits = ResourceLimits::from_env().expect("valid env");

            assert_eq!(limits.max_result_store_bytes, 123_456);
            assert_eq!(limits.result_active_grace_secs, 42);
        },
    );
}

#[test]
fn result_store_surface_budget_follows_the_overridden_response_ceiling() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES", Some("1000")),
            ("SLUGAUDIT_MAX_RESULT_STORE_BYTES", None),
        ],
        || {
            let limits = ResourceLimits::from_env().expect("valid env");

            assert_eq!(limits.max_query_response_bytes, 1000);
            assert_eq!(limits.max_result_store_bytes, 4000);
        },
    );
}

#[test]
fn result_store_surface_own_budget_variable_beats_the_response_ceiling() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES", Some("1000")),
            ("SLUGAUDIT_MAX_RESULT_STORE_BYTES", Some("777")),
        ],
        || {
            assert_eq!(
                ResourceLimits::from_env()
                    .expect("valid env")
                    .max_result_store_bytes,
                777
            );
        },
    );
}

#[test]
fn fail_closed_an_invalid_limit_variable_is_an_error_naming_it_not_its_value() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("12MBsecret")),
        ],
        || {
            let error = ResourceLimits::from_env().expect_err("a typo'd limit must fail startup");

            let message = error.to_string();
            assert!(
                message.starts_with(
                    "SLUGAUDIT_MAX_FILE_BYTES is set but is not valid; expected a value of type u64"
                ),
                "{message}"
            );
            assert!(!message.contains("secret"), "{message}");
        },
    );
}

#[test]
fn fail_closed_a_non_unicode_limit_variable_is_an_error_without_its_value() {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let value = std::ffi::OsString::from_vec(b"secret-\xff".to_vec());
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some(std::ffi::OsString::from("lean"))),
            ("SLUGAUDIT_MAX_WATCH_DEPTH", Some(value)),
        ],
        || {
            let error = ResourceLimits::from_env().expect_err("non-unicode must fail startup");

            let message = error.to_string();
            assert!(
                message.starts_with("SLUGAUDIT_MAX_WATCH_DEPTH is set but is not valid Unicode"),
                "{message}"
            );
            assert!(!message.contains("secret"), "{message}");
        },
    );
}

#[test]
fn fail_closed_undetectable_memory_without_an_explicit_profile_names_the_variable_to_set() {
    let error = base_limits(None, crate::model::system_ram::detect_from_for_test)
        .expect_err("adaptive default needs memory");

    assert!(
        matches!(error, EnvConfigError::MemoryUndetectable { .. }),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("set SLUGAUDIT_PROFILE to `lean` or `audit`"),
        "{error}"
    );
}

#[test]
fn fail_closed_an_explicit_profile_runs_without_detectable_memory() {
    let limits = base_limits(
        Some(crate::model::AuditProfile::Lean),
        crate::model::system_ram::detect_from_for_test,
    )
    .expect("lean needs no detection");

    assert_eq!(limits.max_file_bytes, 32 * 1024 * 1024);
}

#[test]
fn fail_closed_an_uninstalled_limits_cell_with_an_invalid_env_returns_the_error_and_installs_nothing()
 {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let cell = OnceLock::new();
    temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("0")),
        ],
        || {
            let error = install_limits(&cell).expect_err("a zero limit must not install");

            assert!(
                matches!(&error, EnvConfigError::Zero { variable } if variable == "SLUGAUDIT_MAX_PARSE_BYTES"),
                "{error:?}"
            );
            assert!(cell.get().is_none(), "an invalid env must install nothing");
        },
    );
}

#[test]
fn an_uninstalled_limits_cell_with_a_valid_env_installs_once_and_keeps_the_first_value() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let cell = OnceLock::new();
    let first = temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_PARSE_BYTES", Some("4096")),
        ],
        || install_limits(&cell).map(|limits| limits.max_parse_bytes),
    )
    .expect("valid env installs");
    assert_eq!(first, 4096);

    // The environment is not read again once the limits are installed.
    let second = temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("0"), || {
        install_limits(&cell).map(|limits| limits.max_parse_bytes)
    })
    .expect("installed limits are returned without re-reading the env");
    assert_eq!(second, 4096);
}

#[test]
fn the_default_limits_ignore_the_environment_and_the_host_memory() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let baseline = ResourceLimits::default();

    let under_env = temp_env::with_vars(
        [
            ("SLUGAUDIT_PROFILE", Some("lean")),
            ("SLUGAUDIT_MAX_FILE_BYTES", Some("1")),
        ],
        ResourceLimits::default,
    );

    assert_eq!(under_env, baseline);
    assert_eq!(
        baseline,
        crate::model::profile::adaptive_limits(DEFAULT_BASELINE_MEMORY),
    );
}
