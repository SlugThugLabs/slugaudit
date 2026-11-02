//! Unit tests for `check_performance`.

use super::args::{Options, options};
use super::compare::{Verdict, compare};
use super::estimates;
use super::estimates::{BaselineEntry, load_baseline};
use super::format::{fmt_ns, table_line};
use super::record::{baseline_json, unmeasured};
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn fmt_ns_thresholds() {
    assert_eq!(fmt_ns(0), "0 ns");
    assert_eq!(fmt_ns(500), "500 ns");
    assert_eq!(fmt_ns(1_500), "1.50 us");
    assert_eq!(fmt_ns(2_500_000), "2.50 ms");
    assert_eq!(fmt_ns(3_500_000_000), "3.50 s");
}

#[test]
fn load_baseline_parses_object() -> TestResult {
    let raw = r#"{
        "machine": "test",
        "threshold_percent": 20.0,
        "benches": {
            "sync/first": {"median_ns": 100, "budget_ns": 200},
            "parsing/extract_rust": {"median_ns": 50, "budget_ns": null}
        }
    }"#;
    let dir = tempfile::tempdir().check("temp dir")?;
    let tmp = dir.path().join("baseline.json");
    std::fs::write(&tmp, raw).check("baseline file writes")?;
    let map = load_baseline(&tmp).check("parse")?;
    assert_eq!(map["sync/first"].median_ns, 100);
    assert_eq!(map["sync/first"].budget_ns, Some(200));
    assert_eq!(map["parsing/extract_rust"].median_ns, 50);
    assert_eq!(map["parsing/extract_rust"].budget_ns, None);
    Ok(())
}

#[test]
fn load_baseline_rejects_missing_benches() -> TestResult {
    let raw = r#"{"machine": "x", "threshold_percent": 20.0}"#;
    let dir = tempfile::tempdir().check("temp dir")?;
    let tmp = dir.path().join("baseline.json");
    std::fs::write(&tmp, raw).check("baseline file writes")?;
    let err = load_baseline(&tmp).check_err("a baseline without benches is rejected")?;
    assert!(err.contains("missing 'benches' object"), "{err}");
    Ok(())
}

#[test]
fn load_baseline_rejects_a_malformed_budget() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    for budget in [r#""200""#, "-5", "1.5"] {
        let raw = format!(
            r#"{{"benches": {{"sync/first": {{"median_ns": 100, "budget_ns": {budget}}}}}}}"#
        );
        let path = dir.path().join("baseline.json");
        std::fs::write(&path, raw).check("baseline file writes")?;

        let err = load_baseline(&path).check_err("a malformed budget_ns must not be dropped")?;

        assert!(err.contains("budget_ns"), "{budget}: {err}");
    }
    Ok(())
}

#[test]
fn collect_benches_rejects_a_non_finite_or_negative_median() -> TestResult {
    for median in ["-1.0", "1e999"] {
        let root = tempfile::tempdir().check("temp dir")?;
        write_estimates(
            root.path(),
            "g",
            "f",
            &format!(r#"{{"median": {{"point_estimate": {median}}}}}"#),
        )?;

        let result = estimates::collect_benches_in(root.path());

        assert!(result.is_err(), "median {median} must be rejected");
    }
    Ok(())
}

#[test]
fn collect_benches_truncates_a_fractional_median_and_rejects_one_past_u64() -> TestResult {
    for (median, expected) in [("1234.9", 1234_u64), ("0.5", 0), ("-0.0", 0)] {
        let root = tempfile::tempdir().check("temp dir")?;
        write_estimates(
            root.path(),
            "g",
            "f",
            &format!(r#"{{"median": {{"point_estimate": {median}}}}}"#),
        )?;

        let benches = estimates::collect_benches_in(root.path()).check("valid median")?;

        assert_eq!(benches.get("g/f"), Some(&expected), "median {median}");
    }
    for median in ["-0.5", "1.8446744073709552e19"] {
        let root = tempfile::tempdir().check("temp dir")?;
        write_estimates(
            root.path(),
            "g",
            "f",
            &format!(r#"{{"median": {{"point_estimate": {median}}}}}"#),
        )?;

        let error = estimates::collect_benches_in(root.path()).check_err("invalid median")?;

        assert!(
            error.to_string().contains("is not a valid time in ns"),
            "median {median}: {error}"
        );
    }
    Ok(())
}

fn write_estimates(root: &std::path::Path, group: &str, func: &str, body: &str) -> TestResult {
    let dir = root.join(group).join(func).join("new");
    std::fs::create_dir_all(&dir).check("estimates dir")?;
    std::fs::write(dir.join("estimates.json"), body).check("estimates file")?;
    Ok(())
}

#[test]
fn collect_benches_reads_every_new_median_keyed_by_group_and_function() -> TestResult {
    let root = tempfile::tempdir().check("temp dir")?;
    write_estimates(
        root.path(),
        "search",
        "fts",
        r#"{"median":{"point_estimate":1500.7}}"#,
    )?;
    write_estimates(
        root.path(),
        "parsing",
        "rust",
        r#"{"median":{"point_estimate":42}}"#,
    )?;

    let benches = estimates::collect_benches_in(root.path()).check("collects")?;

    assert_eq!(benches.get("search/fts"), Some(&1500));
    assert_eq!(benches.get("parsing/rust"), Some(&42));
    assert_eq!(benches.len(), 2);
    Ok(())
}

#[test]
fn collect_benches_fails_on_a_malformed_estimates_file_instead_of_dropping_the_bench() -> TestResult
{
    let root = tempfile::tempdir().check("temp dir")?;
    write_estimates(
        root.path(),
        "search",
        "fts",
        r#"{"median":{"point_estimate":10}}"#,
    )?;
    write_estimates(root.path(), "search", "broken", "{not json")?;

    let error = estimates::collect_benches_in(root.path()).check_err("malformed file fails")?;

    assert!(error.to_string().contains("broken"), "{error}");
    Ok(())
}

#[test]
fn collect_benches_fails_on_an_estimates_file_with_no_median() -> TestResult {
    let root = tempfile::tempdir().check("temp dir")?;
    write_estimates(
        root.path(),
        "search",
        "fts",
        r#"{"mean":{"point_estimate":10}}"#,
    )?;

    let error = estimates::collect_benches_in(root.path()).check_err("missing median fails")?;

    assert!(error.to_string().contains("median"), "{error}");
    Ok(())
}

fn medians(pairs: &[(&str, u64)]) -> std::collections::BTreeMap<String, u64> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

fn baseline_of(
    pairs: &[(&str, u64, Option<u64>)],
) -> std::collections::BTreeMap<String, BaselineEntry> {
    pairs
        .iter()
        .map(|(k, median_ns, budget_ns)| {
            (
                (*k).to_string(),
                BaselineEntry {
                    median_ns: *median_ns,
                    budget_ns: *budget_ns,
                },
            )
        })
        .collect()
}

#[test]
fn a_bench_within_the_threshold_passes() {
    let measured = medians(&[("search/a", 119)]);
    let baseline = baseline_of(&[("search/a", 100, None)]);

    let result = compare(&measured, &baseline, 20.0);

    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(result.rows[0].verdict, Verdict::Ok);
}

#[test]
fn a_bench_past_the_threshold_regresses() {
    let measured = medians(&[("search/a", 121)]);
    let baseline = baseline_of(&[("search/a", 100, None)]);

    let result = compare(&measured, &baseline, 20.0);

    assert_eq!(result.rows[0].verdict, Verdict::Regressed);
    assert_eq!(result.failures, ["search/a: 21.0% slower than baseline"]);
}

#[test]
fn an_absolute_budget_overrun_outranks_a_regression() {
    let measured = medians(&[("sync/first", 3_000_000)]);
    let baseline = baseline_of(&[("sync/first", 1_000_000, Some(2_000_000))]);

    let result = compare(&measured, &baseline, 20.0);

    assert_eq!(result.rows[0].verdict, Verdict::OverBudget);
    assert_eq!(
        result.failures.len(),
        2,
        "both rules report: {:?}",
        result.failures
    );
}

#[test]
fn a_baselined_bench_that_did_not_run_fails_the_gate() {
    let measured = medians(&[("search/a", 100)]);
    let baseline = baseline_of(&[("search/a", 100, None), ("search/gone", 100, None)]);

    let result = compare(&measured, &baseline, 20.0);

    assert_eq!(result.failures.len(), 1);
    assert!(result.failures[0].starts_with("search/gone: produced no measurements"));
}

#[test]
fn an_unbaselined_bench_warns_and_prints_without_a_ratio() {
    let measured = medians(&[("search/new", 100)]);

    let result = compare(&measured, &baseline_of(&[]), 20.0);

    assert!(result.failures.is_empty());
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.rows[0].verdict, Verdict::New);
    assert!(
        table_line(&result.rows[0]).contains("  new"),
        "row prints its verdict"
    );
}

#[test]
fn a_zero_baseline_median_is_an_infinite_regression() {
    let measured = medians(&[("search/a", 1)]);
    let baseline = baseline_of(&[("search/a", 0, None)]);

    let result = compare(&measured, &baseline, 20.0);

    assert_eq!(result.rows[0].verdict, Verdict::Regressed);
}

#[test]
fn re_recording_keeps_each_hand_set_budget() {
    let measured = medians(&[("sync/first", 5), ("search/a", 7)]);
    let previous = baseline_of(&[("sync/first", 1, Some(99))]);

    let json = baseline_json(&measured, &previous);

    assert_eq!(json["benches"]["sync/first"]["budget_ns"], 99);
    assert_eq!(json["benches"]["sync/first"]["median_ns"], 5);
    assert!(json["benches"]["search/a"]["budget_ns"].is_null());
}

#[test]
fn the_recorded_document_holds_only_what_the_gate_reads() -> TestResult {
    let json = baseline_json(&medians(&[("search/a", 7)]), &baseline_of(&[]));

    let object = json.as_object().check("the baseline is a JSON object")?;

    assert_eq!(object.keys().collect::<Vec<_>>(), ["benches"]);
    Ok(())
}

#[test]
fn a_baselined_bench_the_run_did_not_measure_is_reported_for_a_record() {
    let measured = medians(&[("search/a", 7)]);
    let previous = baseline_of(&[
        ("search/a", 1, None),
        ("sync/first", 1, Some(99)),
        ("parsing/old", 1, None),
    ]);

    assert_eq!(
        unmeasured(&measured, &previous),
        ["parsing/old", "sync/first"]
    );
    assert!(unmeasured(&measured, &baseline_of(&[("search/a", 1, None)])).is_empty());
}

#[test]
fn criterion_output_follows_the_cargo_target_dir() {
    let root = std::path::Path::new("/proj");

    assert_eq!(
        estimates::criterion_dir(root, None),
        std::path::Path::new("/proj/target/criterion")
    );
    assert_eq!(
        estimates::criterion_dir(root, Some(std::ffi::OsStr::new("/shared/target"))),
        std::path::Path::new("/shared/target/criterion")
    );
    assert_eq!(
        estimates::criterion_dir(root, Some(std::ffi::OsStr::new("out"))),
        std::path::Path::new("/proj/out/criterion")
    );
}

#[test]
fn the_cli_rejects_a_non_finite_or_negative_threshold() {
    for raw in ["NaN", "inf", "-1"] {
        let result = options(false, &[raw.to_string()]);

        assert!(result.is_err(), "threshold {raw} must be rejected");
    }
}

#[test]
fn the_cli_builds_options_from_the_record_switch_and_threshold() -> TestResult {
    let run = options(true, &["35".to_string()]).check("valid args parse")?;
    let defaults = options(false, &[]).check("no args parse")?;
    let bad = options(false, &["fast".to_string()]);
    let two = options(false, &["1".to_string(), "2".to_string()]);

    assert_eq!(
        run,
        Options {
            record: true,
            threshold_pct: 35.0
        }
    );
    assert_eq!(
        defaults,
        Options {
            record: false,
            threshold_pct: 20.0
        }
    );
    assert!(
        bad.check_err("non-number is a usage error")?
            .contains("\"fast\"")
    );
    assert!(
        two.check_err("two thresholds are a usage error")?
            .contains("at most one threshold")
    );
    Ok(())
}
