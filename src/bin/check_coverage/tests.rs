use super::*;
use serde_json::json;
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn a_non_finite_or_out_of_range_threshold_is_rejected() {
    for raw in ["NaN", "nan", "inf", "-inf", "-1", "100.5", "abc"] {
        assert!(parse_threshold(raw).is_err(), "{raw} must be rejected");
    }
}

#[test]
fn a_percentage_threshold_is_accepted() {
    assert_eq!(parse_threshold("83"), Ok(83.0));
    assert_eq!(parse_threshold("0"), Ok(0.0));
    assert_eq!(parse_threshold("100"), Ok(100.0));
}

#[test]
fn removing_a_missing_report_succeeds_and_an_unremovable_one_fails() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;

    remove_report(&dir.path().join("absent.json")).check("missing file is fine")?;
    // A directory cannot be removed with remove_file.
    assert!(remove_report(dir.path()).is_err());
    Ok(())
}

#[test]
fn merges_lines_across_data_entries() -> TestResult {
    let cov = json!({
        "data": [
            {"totals": {"lines": {"covered": 100, "count": 200}}},
            {"totals": {"lines": {"covered": 50, "count": 100}}},
        ]
    });
    let (covered, count) = merge_line_totals(&cov).check("merge")?;
    assert_eq!((covered, count), (150, 300));
    Ok(())
}

#[test]
fn returns_none_when_data_missing() {
    let cov = json!({"totals": {"lines": {"covered": 1, "count": 2}}});
    assert_eq!(merge_line_totals(&cov), None);
}

#[test]
fn returns_none_when_totals_missing() {
    let cov = json!({"data": [{"files": []}]});
    assert_eq!(merge_line_totals(&cov), None);
}

#[test]
fn gate_passes_at_or_above_the_threshold() {
    assert_eq!(gate(83, 100, 83.0), Ok((83.0, true)));
    assert_eq!(gate(90, 100, 83.0), Ok((90.0, true)));
}

#[test]
fn gate_fails_below_the_threshold() {
    assert_eq!(gate(82, 100, 83.0), Ok((82.0, false)));
}

#[test]
fn gate_with_no_counted_lines_is_zero_percent() {
    assert_eq!(gate(0, 0, 83.0), Ok((0.0, false)));
    assert_eq!(gate(0, 0, 0.0), Ok((0.0, true)));
}

#[test]
fn gate_rejects_a_total_too_large_for_an_exact_percentage() -> TestResult {
    let too_big = u64::from(u32::MAX) + 1;
    let message = gate(1, too_big, 83.0).check_err("count exceeds u32")?;
    assert!(message.starts_with("FAIL: counted line total 4294967296 is too large"));
    let message = gate(too_big, too_big, 83.0).check_err("covered exceeds u32")?;
    assert!(message.starts_with("FAIL: covered line total 4294967296 is too large"));
    Ok(())
}

#[test]
fn single_entry_passes_through() {
    let cov = json!({"data": [{"totals": {"lines": {"covered": 7, "count": 10}}}]});
    assert_eq!(merge_line_totals(&cov), Some((7, 10)));
}
