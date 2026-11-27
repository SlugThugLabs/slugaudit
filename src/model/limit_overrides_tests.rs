//! Zero refusal for environment limit overrides and warn-once clamping ledger.
//! The warning ledger is per thread under test (process-wide in production), so
//! every test that touches it calls `fresh_ledger()` and counts only the lines
//! carrying its own literal.

use super::*;
use crate::util::test_capture::capture_warns;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn lines_containing(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
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
/// `nonzero_override` in `limit_overrides.rs` must be listed, and nothing else.
#[test]
fn the_zero_refused_list_names_exactly_the_variables_read_with_nonzero_override() -> TestResult {
    let source = include_str!("limit_overrides.rs");
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

    ensure_eq(
        &read,
        &listed,
        "every nonzero_override environment variable in limit_overrides.rs is covered",
    )?;
    Ok(())
}

#[test]
fn fail_closed_a_zero_limit_env_var_is_an_error_naming_only_the_variable() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let _cfg = crate::project::GlobalConfigGuardForTest::none();
    for variable in ZERO_REFUSED {
        let error = temp_env::with_vars(
            [("SLUGAUDIT_PROFILE", Some("lean")), (variable, Some("0"))],
            crate::project::validate_startup_config,
        )
        .check_err("a zero limit must fail startup")?;

        ensure_eq(
            &error.to_string(),
            &format!(
                "{variable} is set to 0, which is not a usable limit; set a positive value or unset it"
            ),
            "zero environment override error names the variable and remedy",
        )?;
        ensure(
            matches!(
                &error,
                crate::project::ProjectLimitsError::Env(EnvConfigError::Zero { variable: named })
                    if named == variable
            ),
            format!("{error:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn the_same_parse_clamp_is_reported_once_however_often_clamped() -> TestResult {
    fresh_ledger().check("reset the ledger")?;
    let clamp_once = |parse_bytes: u64| -> TestResult<ResourceLimits> {
        let mut limits = ResourceLimits {
            max_file_bytes: 700_001,
            max_parse_bytes: parse_bytes,
            ..ResourceLimits::default()
        };
        limits.clamp_parse_to_file_bytes().check("clamp parse")?;
        Ok(limits)
    };

    let (first_batch, logs) = capture_warns(|| -> TestResult<ResourceLimits> {
        let first = clamp_once(900_009)?;
        let second = clamp_once(900_009)?;
        ensure_eq(&first, &second, "repeated clamp yields identical limits")?;
        Ok(second)
    })?;
    let limits = first_batch?;
    let (_, more_logs) = capture_warns(|| clamp_once(900_009))?;
    let (_, changed_logs) = capture_warns(|| clamp_once(900_011))?;

    ensure_eq(
        &limits.max_parse_bytes,
        &700_001,
        "max_parse_bytes clamps down to max_file_bytes",
    )?;
    ensure_eq(&lines_containing(&logs, "requested=900009"), &1, &logs)?;
    ensure_eq(
        &lines_containing(&more_logs, "requested=900009"),
        &0,
        &more_logs,
    )?;
    ensure_eq(
        &lines_containing(&changed_logs, "requested=900011"),
        &1,
        &changed_logs,
    )?;
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

    ensure_eq(
        &clamped?,
        &[SQLITE_MAX_VALUE_BYTES; 3],
        "values above SQLITE_MAX_VALUE_BYTES clamp to SQLITE_MAX_VALUE_BYTES",
    )?;
    ensure_eq(
        &lines_containing(&logs, "setting=\"clamp test setting\""),
        &2,
        &logs,
    )?;
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
    ensure(unwound.is_err(), "the holder must have unwound")?;

    let error = clamp_sqlite_value("poison test setting", 2_000_000_000)
        .check_err("a poisoned ledger is an error")?;

    ensure_eq(
        &error.to_string(),
        &"mutex limit_override_warnings was poisoned by a panic in an earlier holder",
        "poisoned warning ledger error names limit_override_warnings",
    )?;
    Ok(())
}

#[test]
fn a_value_within_sqlite_max_passes_through_without_a_warning() -> TestResult {
    fresh_ledger().check("reset the ledger")?;

    let (value, logs) = capture_warns(|| clamp_sqlite_value("within test setting", 1_000_000_000))?;

    ensure_eq(
        &value.check("clamp")?,
        &1_000_000_000,
        "value at SQLITE_MAX_VALUE_BYTES passes through unchanged",
    )?;
    ensure_eq(&lines_containing(&logs, "within test setting"), &0, &logs)?;
    Ok(())
}
