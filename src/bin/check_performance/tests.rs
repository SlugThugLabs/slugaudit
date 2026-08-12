//! Unit tests for `check_performance`.

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
    std::fs::write(&tmp, raw).unwrap();
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
    std::fs::write(&tmp, raw).unwrap();
    let err = load_baseline(&tmp).unwrap_err();
    assert!(err.contains("missing 'benches' object"), "{err}");
    let _ = std::fs::remove_file(&tmp);
}
