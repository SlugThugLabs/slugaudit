//! Unit tests for `check_performance`.

use super::args::{Options, options};
use super::compare::{Verdict, compare};
use super::estimates;
use super::estimates::{BaselineEntry, load_baseline};
use super::format::{fmt_ns, table_line};
use super::record::{baseline_json, unmeasured};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn fmt_ns_thresholds() -> TestResult {
    ensure_eq(&fmt_ns(0), &"0 ns", "fmt_ns(0) == \"0 ns\"")?;
    ensure_eq(&fmt_ns(500), &"500 ns", "fmt_ns(500) == \"500 ns\"")?;
    ensure_eq(&fmt_ns(1_500), &"1.50 us", "fmt_ns(1_500) == \"1.50 us\"")?;
    ensure_eq(
        &fmt_ns(2_500_000),
        &"2.50 ms",
        "fmt_ns(2_500_000) == \"2.50 ms\"",
    )?;
    ensure_eq(
        &fmt_ns(3_500_000_000),
        &"3.50 s",
        "fmt_ns(3_500_000_000) == \"3.50 s\"",
    )?;
    Ok(())
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
    ensure_eq(
        &map.get("sync/first").check("map has sync/first")?.median_ns,
        &100,
        "map[\"sync/first\"].median_ns == 100",
    )?;
    ensure_eq(
        &map.get("sync/first").check("map has sync/first")?.budget_ns,
        &Some(200),
        "map[\"sync/first\"].budget_ns == Some(200)",
    )?;
    ensure_eq(
        &map.get("parsing/extract_rust")
            .check("map has parsing/extract_rust")?
            .median_ns,
        &50,
        "map[\"parsing/extract_rust\"].median_ns == 50",
    )?;
    ensure_eq(
        &map.get("parsing/extract_rust")
            .check("map has parsing/extract_rust")?
            .budget_ns,
        &None,
        "map[\"parsing/extract_rust\"].budget_ns == None",
    )?;
    Ok(())
}

#[test]
fn load_baseline_rejects_missing_benches() -> TestResult {
    let raw = r#"{"machine": "x", "threshold_percent": 20.0}"#;
    let dir = tempfile::tempdir().check("temp dir")?;
    let tmp = dir.path().join("baseline.json");
    std::fs::write(&tmp, raw).check("baseline file writes")?;
    let err = load_baseline(&tmp).check_err("a baseline without benches is rejected")?;
    ensure(err.contains("missing 'benches' object"), &err)?;
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

        ensure(err.contains("budget_ns"), format!("{budget}: {err}"))?;
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

        ensure(result.is_err(), format!("median {median} must be rejected"))?;
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

        ensure_eq(
            &benches.get("g/f"),
            &Some(&expected),
            format!("median {median}"),
        )?;
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

        ensure(
            error.to_string().contains("is not a valid time in ns"),
            format!("median {median}: {error}"),
        )?;
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

    ensure_eq(
        &benches.get("search/fts"),
        &Some(&1500),
        "benches.get(\"search/fts\") == Some(&1500)",
    )?;
    ensure_eq(
        &benches.get("parsing/rust"),
        &Some(&42),
        "benches.get(\"parsing/rust\") == Some(&42)",
    )?;
    ensure_eq(&benches.len(), &2, "benches.len() == 2")?;
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

    ensure(error.to_string().contains("broken"), format!("{error}"))?;
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

    ensure(error.to_string().contains("median"), format!("{error}"))?;
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
fn a_bench_within_the_threshold_passes() -> TestResult {
    let measured = medians(&[("search/a", 119)]);
    let baseline = baseline_of(&[("search/a", 100, None)]);

    let result = compare(&measured, &baseline, 20.0);

    ensure(result.failures.is_empty(), format!("{:?}", result.failures))?;
    ensure_eq(
        &result.rows.first().check("first rows entry")?.verdict,
        &Verdict::Ok,
        "result.rows[0].verdict == Verdict::Ok",
    )?;
    Ok(())
}

#[test]
fn a_bench_past_the_threshold_regresses() -> TestResult {
    let measured = medians(&[("search/a", 121)]);
    let baseline = baseline_of(&[("search/a", 100, None)]);

    let result = compare(&measured, &baseline, 20.0);

    ensure_eq(
        &result.rows.first().check("first rows entry")?.verdict,
        &Verdict::Regressed,
        "result.rows[0].verdict == Verdict::Regressed",
    )?;
    ensure_eq(
        &result.failures,
        &["search/a: 21.0% slower than baseline"],
        "result.failures == [\"search/a: 21.0% slower than baseline\"]",
    )?;
    Ok(())
}

#[test]
fn an_absolute_budget_overrun_outranks_a_regression() -> TestResult {
    let measured = medians(&[("sync/first", 3_000_000)]);
    let baseline = baseline_of(&[("sync/first", 1_000_000, Some(2_000_000))]);

    let result = compare(&measured, &baseline, 20.0);

    ensure_eq(
        &result.rows.first().check("first rows entry")?.verdict,
        &Verdict::OverBudget,
        "result.rows[0].verdict == Verdict::OverBudget",
    )?;
    ensure_eq(
        &result.failures.len(),
        &2,
        format!("both rules report: {:?}", result.failures),
    )?;
    Ok(())
}

#[test]
fn a_baselined_bench_that_did_not_run_fails_the_gate() -> TestResult {
    let measured = medians(&[("search/a", 100)]);
    let baseline = baseline_of(&[("search/a", 100, None), ("search/gone", 100, None)]);

    let result = compare(&measured, &baseline, 20.0);

    ensure_eq(&result.failures.len(), &1, "result.failures.len() == 1")?;
    ensure(
        result
            .failures
            .first()
            .check("first failures entry")?
            .starts_with("search/gone: produced no measurements"),
        "result.failures[0].starts_with(\"search/gone: produced no measurements\")",
    )?;
    Ok(())
}

#[test]
fn an_unbaselined_bench_warns_and_prints_without_a_ratio() -> TestResult {
    let measured = medians(&[("search/new", 100)]);

    let result = compare(&measured, &baseline_of(&[]), 20.0);

    ensure(result.failures.is_empty(), "result.failures.is_empty()")?;
    ensure_eq(&result.warnings.len(), &1, "result.warnings.len() == 1")?;
    ensure_eq(
        &result.rows.first().check("first rows entry")?.verdict,
        &Verdict::New,
        "result.rows[0].verdict == Verdict::New",
    )?;
    ensure(
        table_line(result.rows.first().check("first rows entry")?).contains("  new"),
        "row prints its verdict",
    )?;
    Ok(())
}

#[test]
fn a_zero_baseline_median_is_an_infinite_regression() -> TestResult {
    let measured = medians(&[("search/a", 1)]);
    let baseline = baseline_of(&[("search/a", 0, None)]);

    let result = compare(&measured, &baseline, 20.0);

    ensure_eq(
        &result.rows.first().check("first rows entry")?.verdict,
        &Verdict::Regressed,
        "result.rows[0].verdict == Verdict::Regressed",
    )?;
    Ok(())
}

#[test]
fn re_recording_keeps_each_hand_set_budget() -> TestResult {
    let measured = medians(&[("sync/first", 5), ("search/a", 7)]);
    let previous = baseline_of(&[("sync/first", 1, Some(99))]);

    let json = baseline_json(&measured, &previous);

    ensure_eq(
        &bench_field(&json, "sync/first", "budget_ns"),
        &Some(&serde_json::Value::from(99)),
        "json[\"benches\"][\"sync/first\"][\"budget_ns\"] == 99",
    )?;
    ensure_eq(
        &bench_field(&json, "sync/first", "median_ns"),
        &Some(&serde_json::Value::from(5)),
        "json[\"benches\"][\"sync/first\"][\"median_ns\"] == 5",
    )?;
    ensure(
        bench_field(&json, "search/a", "budget_ns").is_none_or(serde_json::Value::is_null),
        "json[\"benches\"][\"search/a\"][\"budget_ns\"].is_null()",
    )?;
    Ok(())
}

#[test]
fn the_recorded_document_holds_only_what_the_gate_reads() -> TestResult {
    let json = baseline_json(&medians(&[("search/a", 7)]), &baseline_of(&[]));

    let object = json.as_object().check("the baseline is a JSON object")?;

    ensure_eq(
        &object.keys().collect::<Vec<_>>(),
        &["benches"],
        "object.keys().collect::<Vec<_>>() == [\"benches\"]",
    )?;
    Ok(())
}

#[test]
fn a_baselined_bench_the_run_did_not_measure_is_reported_for_a_record() -> TestResult {
    let measured = medians(&[("search/a", 7)]);
    let previous = baseline_of(&[
        ("search/a", 1, None),
        ("sync/first", 1, Some(99)),
        ("parsing/old", 1, None),
    ]);

    ensure_eq(
        &unmeasured(&measured, &previous),
        &["parsing/old", "sync/first"],
        "unmeasured(&measured, &previous) == [\"parsing/old\", \"sync/first\"]",
    )?;
    ensure(
        unmeasured(&measured, &baseline_of(&[("search/a", 1, None)])).is_empty(),
        "unmeasured(&measured, &baseline_of(&[(\"search/a\", 1, None)])).is_empty()",
    )?;
    Ok(())
}

#[test]
fn criterion_output_follows_the_cargo_target_dir() -> TestResult {
    let root = std::path::Path::new("/proj");

    ensure_eq(
        &estimates::criterion_dir(root, None),
        &std::path::Path::new("/proj/target/criterion"),
        "estimates::criterion_dir(root, None) == std::path::Path::new(\"/proj/target/criterion\")",
    )?;
    ensure_eq(
        &estimates::criterion_dir(root, Some(std::ffi::OsStr::new("/shared/target"))),
        &std::path::Path::new("/shared/target/criterion"),
        "estimates::criterion_dir(root, Some(std::ffi::OsStr::new(\"/shared/target\"))) == std::path::Path::new(\"/shared/target/criterion\")",
    )?;
    ensure_eq(
        &estimates::criterion_dir(root, Some(std::ffi::OsStr::new("out"))),
        &std::path::Path::new("/proj/out/criterion"),
        "estimates::criterion_dir(root, Some(std::ffi::OsStr::new(\"out\"))) == std::path::Path::new(\"/proj/out/criterion\")",
    )?;
    Ok(())
}

#[test]
fn the_cli_rejects_a_non_finite_or_negative_threshold() -> TestResult {
    for raw in ["NaN", "inf", "-1"] {
        let result = options(false, &[raw.to_string()]);

        ensure(result.is_err(), format!("threshold {raw} must be rejected"))?;
    }
    Ok(())
}

#[test]
fn the_cli_builds_options_from_the_record_switch_and_threshold() -> TestResult {
    let run = options(true, &["35".to_string()]).check("valid args parse")?;
    let defaults = options(false, &[]).check("no args parse")?;
    let bad = options(false, &["fast".to_string()]);
    let two = options(false, &["1".to_string(), "2".to_string()]);

    ensure_eq(
        &run,
        &Options {
            record: true,
            threshold_pct: 35.0,
        },
        "run == Options { record: true, threshold_pct: 35.0 }",
    )?;
    ensure_eq(
        &defaults,
        &Options {
            record: false,
            threshold_pct: 20.0,
        },
        "defaults == Options { record: false, threshold_pct: 20.0 }",
    )?;
    ensure(
        bad.check_err("non-number is a usage error")?
            .contains("\"fast\""),
        "bad.check_err(\"non-number is a usage error\")? .contains(\"\\\"fast\\\"\")",
    )?;
    ensure(
        two.check_err("two thresholds are a usage error")?
            .contains("at most one threshold"),
        "two.check_err(\"two thresholds are a usage error\")? .contains(\"at most one threshold\")",
    )?;
    Ok(())
}

/// `json["benches"][bench][field]`, or `None` when any level is absent.
fn bench_field<'a>(
    json: &'a serde_json::Value,
    bench: &str,
    field: &str,
) -> Option<&'a serde_json::Value> {
    json.get("benches")?.get(bench)?.get(field)
}
