//! Unit tests for `check_performance`.

use super::estimates;
use super::estimates::{BaselineEntry, load_baseline};
use super::format::fmt_ns;

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
