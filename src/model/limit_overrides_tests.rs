//! Zero refusal and warn-once for limit overrides. The warning ledger is
//! process-wide, so every test that touches it takes `fresh_ledger()` (one
//! lock, ledger emptied) and counts only the lines carrying its own literal.

use super::*;
use crate::project::ProjectConfig;
use crate::store::test_capture::capture_warns;

fn lines_containing(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
}

/// Loads the limits from the environment twice under one capture.
fn load_twice() -> (ResourceLimits, String) {
    capture_warns(|| {
        let first = ResourceLimits::from_env();
        let second = ResourceLimits::from_env();
        assert_eq!(first, second, "the same environment builds the same limits");
        second
    })
}

#[test]
fn a_zero_max_parse_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_BYTES", Some("0"), || {
        let (limits, logs) = load_twice();

        assert_eq!(limits.max_parse_bytes, 2 * 1024 * 1024);
        assert_eq!(
            lines_containing(&logs, "setting=\"SLUGAUDIT_MAX_PARSE_BYTES\""),
            1,
            "{logs}"
        );
    });
}

#[test]
fn a_zero_max_parse_wall_clock_ms_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS", Some("0"), || {
        let (limits, logs) = load_twice();

        assert_eq!(limits.max_parse_wall_clock_ms, 5_000);
        assert_eq!(
            lines_containing(&logs, "setting=\"SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS\""),
            1,
            "{logs}"
        );
    });
}

#[test]
fn a_zero_max_structure_query_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_var("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES", Some("0"), || {
        let (limits, logs) = load_twice();

        assert_eq!(limits.max_structure_query_bytes, 16 * 1024);
        assert_eq!(
            lines_containing(&logs, "setting=\"SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES\""),
            1,
            "{logs}"
        );
    });
}

/// Applies `config` to the defaults twice under one capture.
fn apply_twice(config: &ProjectConfig) -> (ResourceLimits, String) {
    capture_warns(|| {
        let first = config.apply_to_limits(ResourceLimits::default());
        let second = config.apply_to_limits(ResourceLimits::default());
        assert_eq!(first, second);
        second
    })
}

#[test]
fn a_zero_config_max_parse_bytes_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_parse_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config);

    assert_eq!(limits.max_parse_bytes, 2 * 1024 * 1024);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_parse_bytes\""),
        1,
        "{logs}"
    );
}

#[test]
fn a_zero_config_max_parse_wall_clock_ms_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_parse_wall_clock_ms: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config);

    assert_eq!(limits.max_parse_wall_clock_ms, 5_000);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_parse_wall_clock_ms\""),
        1,
        "{logs}"
    );
}

#[test]
fn a_zero_config_max_structure_query_bytes_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_structure_query_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config);

    assert_eq!(limits.max_structure_query_bytes, 16 * 1024);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_structure_query_bytes\""),
        1,
        "{logs}"
    );
}

#[test]
fn a_zero_config_value_keeps_what_the_environment_already_set() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_parse_bytes: Some(0),
        ..Default::default()
    };
    let from_env = ResourceLimits {
        max_parse_bytes: 123_456,
        ..ResourceLimits::default()
    };

    let applied = config.apply_to_limits(from_env);

    assert_eq!(applied.max_parse_bytes, 123_456);
}

#[test]
fn the_same_clamp_is_reported_once_however_often_the_limits_are_built() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_file_bytes: Some(700_001),
        max_parse_bytes: Some(900_009),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config);
    let (_, more_logs) = apply_twice(&config);

    assert_eq!(limits.max_parse_bytes, 700_001);
    assert_eq!(lines_containing(&logs, "requested=900009"), 1, "{logs}");
    assert_eq!(
        lines_containing(&more_logs, "requested=900009"),
        0,
        "{more_logs}"
    );
}

#[test]
fn a_clamp_with_a_changed_configured_value_is_reported_again() {
    let _ledger = fresh_ledger();
    let configured = |parse_bytes| ProjectConfig {
        max_file_bytes: Some(700_002),
        max_parse_bytes: Some(parse_bytes),
        ..Default::default()
    };

    let (_, first) = apply_twice(&configured(900_010));
    let (_, changed) = apply_twice(&configured(900_011));

    assert_eq!(lines_containing(&first, "requested=900010"), 1, "{first}");
    assert_eq!(
        lines_containing(&changed, "requested=900011"),
        1,
        "{changed}"
    );
}

// --- Zero refusal for every other limit -------------------------------------
//
// The environment tests build over the lean profile (a fixed baseline: the
// adaptive default scales with host memory) and compare against it.

fn lean() -> ResourceLimits {
    crate::model::limits_for_profile(crate::model::AuditProfile::Lean)
}

/// Builds the limits twice with `variable` set to `0` over the lean profile.
/// Returns them, the number of warning lines naming `variable`, and the log.
fn zero_env(variable: &str) -> (ResourceLimits, usize, String) {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    temp_env::with_vars(
        [("SLUGAUDIT_PROFILE", Some("lean")), (variable, Some("0"))],
        || {
            let (limits, logs) = load_twice();
            let warnings = lines_containing(&logs, &format!("setting=\"{variable}\""));
            (limits, warnings, logs)
        },
    )
}

#[test]
fn a_zero_max_file_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_FILE_BYTES");

    assert_eq!(limits.max_file_bytes, lean().max_file_bytes);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_total_import_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES");

    assert_eq!(limits.max_total_import_bytes, lean().max_total_import_bytes);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_query_response_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES");

    assert_eq!(
        limits.max_query_response_bytes,
        lean().max_query_response_bytes
    );
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_default_page_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_DEFAULT_PAGE_BYTES");

    assert_eq!(limits.default_page_bytes, lean().default_page_bytes);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_query_sql_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_QUERY_SQL_BYTES");

    assert_eq!(limits.max_query_sql_bytes, lean().max_query_sql_bytes);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_query_vm_steps_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_QUERY_VM_STEPS");

    assert_eq!(limits.max_query_vm_steps, lean().max_query_vm_steps);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_query_wall_clock_secs_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS");

    assert_eq!(limits.max_query_wall_clock, lean().max_query_wall_clock);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_query_value_bytes_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_QUERY_VALUE_BYTES");

    assert_eq!(limits.max_query_value_bytes, lean().max_query_value_bytes);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_structure_matches_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_STRUCTURE_MATCHES");

    assert_eq!(limits.max_structure_matches, lean().max_structure_matches);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_structure_execution_time_secs_env_var_is_refused_with_one_warning_and_keeps_the_default()
 {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS");

    assert_eq!(
        limits.max_structure_execution_time,
        lean().max_structure_execution_time
    );
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_sync_wall_clock_secs_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS");

    assert_eq!(limits.max_sync_wall_clock, lean().max_sync_wall_clock);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_watch_entries_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_WATCH_ENTRIES");

    assert_eq!(limits.max_watch_entries, lean().max_watch_entries);
    assert_eq!(warnings, 1, "{logs}");
}

#[test]
fn a_zero_max_watch_depth_env_var_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let (limits, warnings, logs) = zero_env("SLUGAUDIT_MAX_WATCH_DEPTH");

    assert_eq!(limits.max_watch_depth, lean().max_watch_depth);
    assert_eq!(warnings, 1, "{logs}");
}

/// Applies `config` over the lean profile twice under one capture.
fn apply_twice_over_lean(config: &ProjectConfig) -> (ResourceLimits, String) {
    capture_warns(|| {
        let first = config.apply_to_limits(lean());
        let second = config.apply_to_limits(lean());
        assert_eq!(first, second);
        second
    })
}

#[test]
fn a_zero_config_max_file_bytes_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_file_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config);

    assert_eq!(limits.max_file_bytes, lean().max_file_bytes);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_file_bytes\""),
        1,
        "{logs}"
    );
}

#[test]
fn a_zero_config_max_total_import_bytes_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        max_total_import_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config);

    assert_eq!(limits.max_total_import_bytes, lean().max_total_import_bytes);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_total_import_bytes\""),
        1,
        "{logs}"
    );
}

#[test]
fn a_zero_config_default_page_bytes_is_refused_with_one_warning_and_keeps_the_default() {
    let _ledger = fresh_ledger();
    let config = ProjectConfig {
        default_page_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config);

    assert_eq!(limits.default_page_bytes, lean().default_page_bytes);
    assert_eq!(
        lines_containing(&logs, "setting=\"config default_page_bytes\""),
        1,
        "{logs}"
    );
}
