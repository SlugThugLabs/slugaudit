//! Unit tests for `check_performance`.

use super::args::{Command, Options, parse};
use super::compare::{Verdict, compare};
use super::estimates;
use super::estimates::{BaselineEntry, load_baseline};
use super::format::{fmt_ns, table_line};
use super::record::baseline_json;

#[test]
fn fmt_ns_thresholds() {
    assert_eq!(fmt_ns(0), "0 ns");
    assert_eq!(fmt_ns(500), "500 ns");
    assert_eq!(fmt_ns(1_500), "1.50 us");
    assert_eq!(fmt_ns(2_500_000), "2.50 ms");
    assert_eq!(fmt_ns(3_500_000_000), "3.50 s");
}

#[test]
fn load_baseline_parses_object() {
    let raw = r#"{
        "machine": "test",
        "threshold_percent": 20.0,
        "benches": {
            "sync/first": {"median_ns": 100, "budget_ns": 200},
            "parsing/extract_rust": {"median_ns": 50, "budget_ns": null}
        }
    }"#;
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "slugaudit-check-perf-baseline-{}.json",
        std::process::id()
    ));
    std::fs::write(&tmp, raw).expect("baseline file writes");
    let map = load_baseline(&tmp).expect("parse");
    assert_eq!(map["sync/first"].median_ns, 100);
    assert_eq!(map["sync/first"].budget_ns, Some(200));
    assert_eq!(map["parsing/extract_rust"].median_ns, 50);
    assert_eq!(map["parsing/extract_rust"].budget_ns, None);
    let _ = std::fs::remove_file(&tmp);
    let _ = BaselineEntry::default();
}

#[test]
fn load_baseline_rejects_missing_benches() {
    let raw = r#"{"machine": "x", "threshold_percent": 20.0}"#;
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "slugaudit-check-perf-baseline-bad-{}.json",
        std::process::id()
    ));
    std::fs::write(&tmp, raw).expect("baseline file writes");
    let err = load_baseline(&tmp).unwrap_err();
    assert!(err.contains("missing 'benches' object"), "{err}");
    let _ = std::fs::remove_file(&tmp);
}

fn write_estimates(root: &std::path::Path, group: &str, func: &str, body: &str) {
    let dir = root.join(group).join(func).join("new");
    std::fs::create_dir_all(&dir).expect("estimates dir");
    std::fs::write(dir.join("estimates.json"), body).expect("estimates file");
}

#[test]
fn collect_benches_reads_every_new_median_keyed_by_group_and_function() {
    let root = tempfile::tempdir().expect("temp dir");
    write_estimates(
        root.path(),
        "search",
        "fts",
        r#"{"median":{"point_estimate":1500.7}}"#,
    );
    write_estimates(
        root.path(),
        "parsing",
        "rust",
        r#"{"median":{"point_estimate":42}}"#,
    );

    let benches = estimates::collect_benches_in(root.path()).expect("collects");

    assert_eq!(benches.get("search/fts"), Some(&1500));
    assert_eq!(benches.get("parsing/rust"), Some(&42));
    assert_eq!(benches.len(), 2);
}

#[test]
fn collect_benches_fails_on_a_malformed_estimates_file_instead_of_dropping_the_bench() {
    let root = tempfile::tempdir().expect("temp dir");
    write_estimates(
        root.path(),
        "search",
        "fts",
        r#"{"median":{"point_estimate":10}}"#,
    );
    write_estimates(root.path(), "search", "broken", "{not json");

    let error = estimates::collect_benches_in(root.path()).expect_err("malformed file fails");

    assert!(error.to_string().contains("broken"), "{error}");
}

#[test]
fn collect_benches_fails_on_an_estimates_file_with_no_median() {
    let root = tempfile::tempdir().expect("temp dir");
    write_estimates(
        root.path(),
        "search",
        "fts",
        r#"{"mean":{"point_estimate":10}}"#,
    );

    let error = estimates::collect_benches_in(root.path()).expect_err("missing median fails");

    assert!(error.to_string().contains("median"), "{error}");
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

    let json = baseline_json(&measured, &previous, 20.0, "m");

    assert_eq!(json["benches"]["sync/first"]["budget_ns"], 99);
    assert_eq!(json["benches"]["sync/first"]["median_ns"], 5);
    assert!(json["benches"]["search/a"]["budget_ns"].is_null());
    assert_eq!(json["machine"], "m");
}

#[test]
fn the_cli_parses_record_threshold_and_help() {
    let args = |list: &[&str]| parse(list.iter().map(|s| (*s).to_string()));

    let run = args(&["--record", "35"]).expect("valid args parse");
    let help = args(&["--help"]).expect("help parses");
    let bad = args(&["fast"]);

    assert_eq!(
        run,
        Command::Run(Options {
            record: true,
            threshold_pct: 35.0
        })
    );
    assert_eq!(help, Command::Help);
    assert!(
        bad.expect_err("non-number is a usage error")
            .contains("\"fast\"")
    );
}
