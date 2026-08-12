//! Performance regression gate.
// slugaudit-line-exception: approved-by=agent; reason=criterion invocation argv, per-row regression comparison with budget tracking, and the verdict/warning/failure printer share one process so the bin's single user-visible CLI output stays coherent; extracting the comparison loop would force cross-module State for the baseline-entry budget fields and duplicate the threshold + record-mode arg parsing
//!
//! Replaces `tools/check_performance.sh`. Runs the four criterion
//! benches with a reduced sample size (fast enough for CI), parses
//! criterion's median point estimates, and fails when any bench
//! regresses more than the threshold against the committed baseline
//! (`.planning/perf_baseline.json`). The same policy as the coverage
//! gate: the gated numbers are printed explicitly, so there is never a
//! silent failure.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. `cargo
//! bench` is invoked through `std::process::Command`; estimate JSONs
//! are parsed with `serde_json` (already a dependency).
//!
//! Usage: `cargo run --bin check_performance --locked [--record] [threshold-percent]`
//!   --record: regenerate `.planning/perf_baseline.json` from this run.
//!   threshold: regression threshold percent (default 20).

mod estimates;
mod format;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

const BASELINE_PATH: &str = ".planning/perf_baseline.json";
const CRITERION_DIR: &str = "target/criterion";
const DEFAULT_THRESHOLD_PCT: f64 = 20.0;

fn main() -> std::process::ExitCode {
    let mut record = false;
    let mut threshold_pct = DEFAULT_THRESHOLD_PCT;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--record" => record = true,
            "--help" => {
                println!(
                    "Performance regression gate. Usage: check_performance [--record] [threshold_percent]"
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => match other.parse::<f64>() {
                Ok(n) => threshold_pct = n,
                Err(err) => {
                    eprintln!("threshold {other:?} is not a number: {err}");
                    return std::process::ExitCode::from(2);
                }
            },
        }
    }

    use std::path::PathBuf;
    let baseline_path = PathBuf::from(BASELINE_PATH);
    if !record && !baseline_path.exists() {
        eprintln!(
            "FAIL: no baseline at {BASELINE_PATH}; run 'cargo run --bin check_performance -- --record' first"
        );
        return std::process::ExitCode::from(1);
    }

    // Clean stale criterion outputs — never trust the previous run's
    // numbers; only the freshly produced `new/estimates.json` files.
    let _ = std::fs::remove_dir_all(CRITERION_DIR);

    let status = std::process::Command::new("cargo")
        .args([
            "bench",
            "--locked",
            "--bench",
            "discovery",
            "--bench",
            "parsing",
            "--bench",
            "search",
            "--bench",
            "sync",
            "--",
            "--sample-size",
            "10",
            "--warm-up-time",
            "1",
            "--measurement-time",
            "3",
        ])
        .status();
    let status = match status {
        Ok(s) => s,
        Err(err) => {
            eprintln!("FAIL: failed to invoke cargo bench: {err}");
            return std::process::ExitCode::from(1);
        }
    };
    if !status.success() {
        eprintln!("FAIL: cargo bench did not complete: {status}");
        return std::process::ExitCode::from(1);
    }

    let new_benches = match estimates::collect_new_benches() {
        Ok(m) => m,
        Err(err) => {
            eprintln!("FAIL: {err}");
            return std::process::ExitCode::from(1);
        }
    };
    if new_benches.is_empty() {
        eprintln!("FAIL: no criterion estimates found under {CRITERION_DIR}");
        return std::process::ExitCode::from(1);
    }

    let old_benches = if baseline_path.exists() {
        match estimates::load_baseline(&baseline_path) {
            Ok(m) => m,
            Err(err) => {
                eprintln!("FAIL: {err}");
                return std::process::ExitCode::from(1);
            }
        }
    } else {
        Default::default()
    };

    if record {
        let mut benches_map = serde_json::Map::new();
        for (key, median_ns) in &new_benches {
            let budget_ns = old_benches.get(key).and_then(|e| e.budget_ns);
            benches_map.insert(
                key.clone(),
                serde_json::json!({ "median_ns": median_ns, "budget_ns": budget_ns }),
            );
        }
        let out = serde_json::json!({
            "machine": format!(
                "current run (re-recorded {})",
                std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            "threshold_percent": threshold_pct,
            "benches": serde_json::Value::Object(benches_map),
        });
        let serialised = match serde_json::to_string_pretty(&out) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("FAIL: could not serialise baseline JSON: {err}");
                return std::process::ExitCode::from(1);
            }
        };
        if let Some(parent) = baseline_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = std::fs::write(&baseline_path, format!("{serialised}\n")) {
            eprintln!(
                "FAIL: could not write baseline {}: {err}",
                baseline_path.display()
            );
            return std::process::ExitCode::from(1);
        }
        println!(
            "recorded {} benchmarks to {}",
            new_benches.len(),
            baseline_path.display()
        );
        return std::process::ExitCode::SUCCESS;
    }

    let mut failures: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();

    for key in new_benches.keys() {
        if !old_benches.contains_key(key) {
            warnings.push(format!(
                "bench {key} is not in the baseline -- run 'check_performance --record' to add it"
            ));
        }
    }
    for key in old_benches.keys() {
        if !new_benches.contains_key(key) {
            failures.push(format!(
                "{key}: produced no measurements this run (did it skip?) -- the gate cannot check a bench that did not run"
            ));
        }
    }
    for (key, new_median) in &new_benches {
        let Some(old_entry) = old_benches.get(key) else {
            rows.push(Row {
                key: key.clone(),
                baseline_ns: None,
                new_ns: *new_median,
                ratio: None,
                verdict: "new".to_string(),
            });
            continue;
        };
        let old_median = old_entry.median_ns;
        let ratio = if old_median == 0 {
            f64::INFINITY
        } else {
            (*new_median as f64) / (old_median as f64)
        };
        let mut verdict = "ok".to_string();
        if ratio > 1.0 + threshold_pct / 100.0 {
            verdict = "REGRESSED".to_string();
            failures.push(format!(
                "{key}: {:.1}% slower than baseline",
                (ratio - 1.0) * 100.0
            ));
        }
        if let Some(budget) = old_entry.budget_ns
            && *new_median > budget
        {
            verdict = "OVER BUDGET".to_string();
            failures.push(format!(
                "{key}: {:.1} ms exceeds the {:.0} ms budget",
                *new_median as f64 / 1e6,
                budget as f64 / 1e6
            ));
        }
        rows.push(Row {
            key: key.clone(),
            baseline_ns: Some(old_median),
            new_ns: *new_median,
            ratio: Some(ratio),
            verdict,
        });
    }
    rows.sort_by(|a, b| a.key.cmp(&b.key));

    println!(
        "perf regression check (threshold {:.0}%, baseline: {})",
        threshold_pct,
        baseline_path.display()
    );
    println!(
        "{:<32} {:>12} {:>12} {:>7}  verdict",
        "bench", "baseline", "measured", "ratio"
    );
    for row in &rows {
        match (row.baseline_ns, row.ratio) {
            (None, _) => println!(
                "{:<32} {:>12} {:>12} {:>7}  {}",
                row.key,
                "-",
                format::fmt_ns(row.new_ns),
                "-",
                row.verdict
            ),
            (Some(baseline), Some(ratio)) => println!(
                "{:<32} {:>12} {:>12} {:>6.2}x  {}",
                row.key,
                format::fmt_ns(baseline),
                format::fmt_ns(row.new_ns),
                ratio,
                row.verdict
            ),
            _ => {}
        }
    }

    for warning in &warnings {
        println!("WARN: {warning}");
    }
    if !failures.is_empty() {
        for failure in &failures {
            println!("FAIL: {failure}");
        }
        return std::process::ExitCode::from(1);
    }
    println!(
        "PASS: no benchmark regressed more than {:.0}%",
        threshold_pct
    );
    std::process::ExitCode::SUCCESS
}

#[derive(Debug, Clone)]
struct Row {
    key: String,
    baseline_ns: Option<u64>,
    new_ns: u64,
    ratio: Option<f64>,
    verdict: String,
}
