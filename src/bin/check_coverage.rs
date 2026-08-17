//! Line coverage gate.
//!
//! Replaces `tools/check_coverage.sh`. Reads the merged JSON line
//! coverage — the same metric `cargo llvm-cov --fail-under-lines`
//! compares (`totals.lines.percent`) — and fails the run if it is
//! below the threshold. The measured number is printed so the gated
//! value is never ambiguous: the `--summary-only` text table prints
//! region-weighted columns that have been misread as line coverage
//! before, and that printed table is not what the exit check compares
//! against.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. `cargo
//! llvm-cov` is invoked through `std::process::Command`; the JSON
//! report is parsed with `serde_json` (already a dependency).

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    // Default threshold is the measured line coverage as of the
    // 2026-08-12 /coverage-gate-recalibrated decision. The gate's intent
    // is "catch coverage regressions, not block on them", so it tracks
    // measurement rather than an aspirational number. Use `cargo run
    // --quiet --bin check_coverage -- <threshold>` to override for a
    // single run (e.g. CI flavor that wants a harder floor); document
    // any raise in `.planning/DECISIONS.md` so the gate stays honest.
    let threshold_str: String = args.next().unwrap_or_else(|| "83".to_string());
    let threshold: f64 = match threshold_str.parse() {
        Ok(n) => n,
        Err(err) => {
            eprintln!("threshold {threshold_str:?} is not a number: {err}");
            return ExitCode::from(2);
        }
    };

    // Cargo-llvm-cov writes the report to `--output-path=<file>`. Use
    // a stable temp file so the path is predictable and easy to clean
    // up after extraction.
    let report_path = std::env::temp_dir().join("slugaudit-check-coverage.cov.json");
    let _ = std::fs::remove_file(&report_path);

    let status = Command::new("cargo")
        .args(["llvm-cov", "--all-targets", "--all-features", "--json"])
        .arg(format!("--output-path={}", report_path.display()))
        .status();
    let status = match status {
        Ok(s) => s,
        Err(err) => {
            eprintln!("FAIL: failed to invoke cargo llvm-cov: {err}");
            return ExitCode::from(1);
        }
    };
    if !status.success() {
        eprintln!("FAIL: cargo llvm-cov --json did not complete: {status}");
        return ExitCode::from(1);
    }

    let raw = match std::fs::read_to_string(&report_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!(
                "FAIL: coverage report at {} was not written: {err}",
                report_path.display()
            );
            return ExitCode::from(1);
        }
    };
    let cov: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("FAIL: could not parse coverage JSON: {err}");
            let _ = std::fs::remove_file(&report_path);
            return ExitCode::from(1);
        }
    };
    let (covered, count) = match merge_line_totals(&cov) {
        Some(v) => v,
        None => {
            eprintln!("FAIL: coverage JSON missing data[*].totals.lines");
            let _ = std::fs::remove_file(&report_path);
            return ExitCode::from(1);
        }
    };
    let percent = if count == 0 {
        0.0
    } else {
        (covered as f64) / (count as f64) * 100.0
    };
    println!(
        "line coverage: {:.2}% ({} / {} lines, gate {}%)",
        percent, covered, count, threshold
    );
    let verdict = if percent < threshold {
        Some(ExitCode::from(1))
    } else {
        None
    };
    if let Some(code) = verdict {
        println!(
            "FAIL: line coverage {:.2}% is below the {}% gate",
            percent, threshold
        );
        let _ = std::fs::remove_file(&report_path);
        return code;
    }
    let _ = std::fs::remove_file(&report_path);
    ExitCode::SUCCESS
}

/// Merge every data entry: newer cargo-llvm-cov versions can emit one
/// entry per test binary, and the gate must measure the true merged
/// coverage, not whichever binary happens to be first.
fn merge_line_totals(cov: &serde_json::Value) -> Option<(u64, u64)> {
    let data = cov.get("data")?.as_array()?;
    let mut covered: u64 = 0;
    let mut count: u64 = 0;
    for entry in data {
        let lines = entry.get("totals")?.get("lines")?;
        covered += lines.get("covered")?.as_u64()?;
        count += lines.get("count")?.as_u64()?;
    }
    Some((covered, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_lines_across_data_entries() {
        let cov = json!({
            "data": [
                {"totals": {"lines": {"covered": 100, "count": 200}}},
                {"totals": {"lines": {"covered": 50, "count": 100}}},
            ]
        });
        let (covered, count) = merge_line_totals(&cov).expect("merge");
        assert_eq!((covered, count), (150, 300));
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
    fn single_entry_passes_through() {
        let cov = json!({"data": [{"totals": {"lines": {"covered": 7, "count": 10}}}]});
        assert_eq!(merge_line_totals(&cov), Some((7, 10)));
    }
}
