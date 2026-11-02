//! Zero refusal and warn-once for limit overrides. The warning ledger is
//! per thread under test (process-wide in production), so every test that
//! touches it calls `fresh_ledger()` and counts only the lines carrying its own literal.

use super::*;
use crate::project::GlobalConfig;
use crate::store::test_capture::capture_warns;
use slugaudit_test_check::{Check, CheckErr, TestResult};

fn lines_containing(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
}

/// Applies `config` to the defaults twice under one capture.
fn apply_twice(config: &GlobalConfig) -> TestResult<(ResourceLimits, String)> {
    let (applied, logs) = capture_warns(|| -> TestResult<ResourceLimits> {
        let first = config
            .apply_limit_keys(ResourceLimits::default())
            .check("first apply")?;
        let second = config
            .apply_limit_keys(ResourceLimits::default())
            .check("second apply")?;
        assert_eq!(first, second);
        Ok(second)
    })?;
    Ok((applied?, logs))
}

#[test]
fn a_zero_config_max_parse_bytes_is_refused_with_one_warning_and_keeps_the_default() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_parse_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config)?;

    assert_eq!(limits.max_parse_bytes, 2 * 1024 * 1024);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_parse_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_max_parse_wall_clock_ms_is_refused_with_one_warning_and_keeps_the_default()
-> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_parse_wall_clock_ms: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config)?;

    assert_eq!(limits.max_parse_wall_clock_ms, 5_000);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_parse_wall_clock_ms\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_max_structure_query_bytes_is_refused_with_one_warning_and_keeps_the_default()
-> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_structure_query_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config)?;

    assert_eq!(limits.max_structure_query_bytes, 16 * 1024);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_structure_query_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_value_keeps_what_the_environment_already_set() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_parse_bytes: Some(0),
        ..Default::default()
    };
    let from_env = ResourceLimits {
        max_parse_bytes: 123_456,
        ..ResourceLimits::default()
    };

    let applied = config
        .apply_limit_keys(from_env)
        .check("apply the config")?;

    assert_eq!(applied.max_parse_bytes, 123_456);
    Ok(())
}

#[test]
fn the_same_clamp_is_reported_once_however_often_the_limits_are_built() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_file_bytes: Some(700_001),
        max_parse_bytes: Some(900_009),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config)?;
    let (_, more_logs) = apply_twice(&config)?;

    assert_eq!(limits.max_parse_bytes, 700_001);
    assert_eq!(lines_containing(&logs, "requested=900009"), 1, "{logs}");
    assert_eq!(
        lines_containing(&more_logs, "requested=900009"),
        0,
        "{more_logs}"
    );
    Ok(())
}

#[test]
fn a_clamp_with_a_changed_configured_value_is_reported_again() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let configured = |parse_bytes| GlobalConfig {
        max_file_bytes: Some(700_002),
        max_parse_bytes: Some(parse_bytes),
        ..Default::default()
    };

    let (_, first) = apply_twice(&configured(900_010))?;
    let (_, changed) = apply_twice(&configured(900_011))?;

    assert_eq!(lines_containing(&first, "requested=900010"), 1, "{first}");
    assert_eq!(
        lines_containing(&changed, "requested=900011"),
        1,
        "{changed}"
    );
    Ok(())
}

// --- Zero refusal for every other limit -------------------------------------

fn lean() -> TestResult<ResourceLimits> {
    crate::model::limits_for_profile(crate::model::AuditProfile::Lean).check("lean limits")
}

/// Every limit variable whose zero is refused at startup.
const ZERO_REFUSED: [&str; 16] = [
    "SLUGAUDIT_MAX_FILE_BYTES",
    "SLUGAUDIT_MAX_PARSE_BYTES",
    "SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS",
    "SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES",
    "SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES",
    "SLUGAUDIT_DEFAULT_PAGE_BYTES",
    "SLUGAUDIT_MAX_QUERY_SQL_BYTES",
    "SLUGAUDIT_MAX_QUERY_VM_STEPS",
    "SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS",
    "SLUGAUDIT_MAX_QUERY_VALUE_BYTES",
    "SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES",
    "SLUGAUDIT_MAX_STRUCTURE_MATCHES",
    "SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS",
    "SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS",
    "SLUGAUDIT_MAX_RESULT_STORE_BYTES",
    "SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS",
];

/// Keeps [`ZERO_REFUSED`] in sync with the code: every variable read through
/// `nonzero_override` in `env_overrides.rs` must be listed, and nothing else.
#[test]
fn the_zero_refused_list_names_exactly_the_variables_env_overrides_rs_reads_with_nonzero_override()
{
    let source = include_str!("env_overrides.rs");
    let mut read: Vec<&str> = source
        .split("nonzero_override")
        .skip(1)
        .filter_map(|rest| {
            let arguments = rest.split_once('(')?.1.trim_start();
            let name = arguments.strip_prefix('"')?.split_once('"')?.0;
            name.starts_with("SLUGAUDIT_").then_some(name)
        })
        .collect();
    read.sort_unstable();
    let mut listed = ZERO_REFUSED.to_vec();
    listed.sort_unstable();

    assert_eq!(read, listed);
}

#[test]
fn fail_closed_a_zero_limit_env_var_is_an_error_naming_only_the_variable() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    for variable in ZERO_REFUSED {
        let error = temp_env::with_vars(
            [("SLUGAUDIT_PROFILE", Some("lean")), (variable, Some("0"))],
            ResourceLimits::from_env,
        )
        .check_err("a zero limit must fail startup")?;

        assert_eq!(
            error.to_string(),
            format!(
                "{variable} is set to 0, which is not a usable limit; set a positive value or unset it"
            ),
        );
        assert!(
            matches!(&error, EnvConfigError::Zero { variable: named } if named == variable),
            "{error:?}"
        );
    }
    Ok(())
}

/// Applies `config` over the lean profile twice under one capture.
fn apply_twice_over_lean(config: &GlobalConfig) -> TestResult<(ResourceLimits, String)> {
    let first_base = lean()?;
    let second_base = lean()?;
    let (applied, logs) = capture_warns(|| -> TestResult<ResourceLimits> {
        let first = config.apply_limit_keys(first_base).check("first apply")?;
        let second = config.apply_limit_keys(second_base).check("second apply")?;
        assert_eq!(first, second);
        Ok(second)
    })?;
    Ok((applied?, logs))
}

#[test]
fn a_zero_config_max_file_bytes_is_refused_with_one_warning_and_keeps_the_default() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_file_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config)?;

    assert_eq!(limits.max_file_bytes, lean()?.max_file_bytes);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_file_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_max_total_import_bytes_is_refused_with_one_warning_and_keeps_the_default()
-> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_total_import_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config)?;

    assert_eq!(
        limits.max_total_import_bytes,
        lean()?.max_total_import_bytes
    );
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_total_import_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_default_page_bytes_is_refused_with_one_warning_and_keeps_the_default() -> TestResult
{
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        default_page_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config)?;

    assert_eq!(limits.default_page_bytes, lean()?.default_page_bytes);
    assert_eq!(
        lines_containing(&logs, "setting=\"config default_page_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_max_result_store_bytes_is_refused_with_one_warning_and_keeps_the_default()
-> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_result_store_bytes: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config)?;

    assert_eq!(
        limits.max_result_store_bytes,
        lean()?.max_result_store_bytes
    );
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_result_store_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_zero_config_result_active_grace_secs_is_refused_with_one_warning_and_keeps_the_default()
-> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        result_active_grace_secs: Some(0),
        ..Default::default()
    };

    let (limits, logs) = apply_twice_over_lean(&config)?;

    assert_eq!(limits.result_active_grace_secs, 900);
    assert_eq!(
        lines_containing(&logs, "setting=\"config result_active_grace_secs\""),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_value_above_sqlite_max_is_clamped_with_one_warning_per_configured_value() -> TestResult {
    fresh_ledger().check("reset the ledger")?;

    let (clamped, logs) = capture_warns(|| -> TestResult<[u64; 3]> {
        Ok([
            clamp_sqlite_value("clamp test setting", 2_000_000_000).check("first clamp")?,
            clamp_sqlite_value("clamp test setting", 2_000_000_000).check("second clamp")?,
            clamp_sqlite_value("clamp test setting", 3_000_000_000).check("new value")?,
        ])
    })?;

    assert_eq!(clamped?, [SQLITE_MAX_VALUE_BYTES; 3]);
    assert_eq!(
        lines_containing(&logs, "setting=\"clamp test setting\""),
        2,
        "{logs}"
    );
    Ok(())
}

/// A poisoned ledger cannot say whether a warning is due, so the clamp fails
/// instead of guessing.
#[test]
fn a_poisoned_ledger_fails_the_clamp_naming_the_ledger() -> TestResult {
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TestResult {
        REPORTED.with(|ledger| -> TestResult {
            let _guard = ledger.lock().check("lock the ledger")?;
            std::panic::resume_unwind(Box::new("simulated panic inside the critical section"))
        })
    }));
    assert!(unwound.is_err(), "the holder must have unwound");

    let error = clamp_sqlite_value("poison test setting", 2_000_000_000)
        .check_err("a poisoned ledger is an error")?;

    assert_eq!(
        error.to_string(),
        "mutex limit_override_warnings was poisoned by a panic in an earlier holder"
    );
    Ok(())
}

#[test]
fn a_value_within_sqlite_max_passes_through_without_a_warning() -> TestResult {
    fresh_ledger().check("reset the ledger")?;

    let (value, logs) = capture_warns(|| clamp_sqlite_value("within test setting", 1_000_000_000))?;

    assert_eq!(value.check("clamp")?, 1_000_000_000);
    assert_eq!(lines_containing(&logs, "within test setting"), 0, "{logs}");
    Ok(())
}

#[test]
fn a_config_max_file_bytes_above_sqlite_max_warns_once_across_calls() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let config = GlobalConfig {
        max_file_bytes: Some(2_000_000_000),
        ..Default::default()
    };

    let (limits, logs) = apply_twice(&config)?;

    assert_eq!(limits.max_file_bytes, SQLITE_MAX_VALUE_BYTES);
    assert_eq!(
        lines_containing(&logs, "setting=\"config max_file_bytes\""),
        1,
        "{logs}"
    );
    Ok(())
}
