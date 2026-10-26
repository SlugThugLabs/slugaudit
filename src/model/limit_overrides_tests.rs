//! Zero refusal and warn-once for limit overrides. The warning ledger is
//! process-wide, so each test uses a (setting, value) pair no other test
//! uses, and counts only the lines carrying its own literal.

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
