#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

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
//! Scope: the gate measures the shipped product runtime. The dev-only
//! `src/bin/check_*` gate binaries are excluded via
//! `--ignore-filename-regex` — AGENTS.md's "Shipped artifact vs
//! development-only" section says a coverage audit must "not count
//! development-only check binaries as end-user runtime code", and
//! measuring them here while their own test files are excluded from the
//! report left six gate mains permanently at 0% inside the denominator
//! (892 lines, ~8 points of measured coverage). Measured product
//! coverage at the 2026-10-16 scope change: 87.89%; the 83 floor is
//! unchanged (measured-minus-margin). See DECISIONS 2026-10-16.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. `cargo
//! llvm-cov` is invoked through `std::process::Command`; the JSON
//! report is parsed with `serde_json` (already a dependency).

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    // Default threshold is the measured line coverage as of the
    // 2026-08-12 /coverage-gate-recalibrated decision. The gate's intent
    // is "catch coverage regressions, not block on them", so it tracks
    // measurement rather than an aspirational number. Use `cargo run
    // --quiet --bin check_coverage -- <threshold>` to override for a
    // single run (e.g. CI flavor that wants a harder floor); document
    // any raise in `.planning/archive/DECISIONS.md` so the gate stays honest.
    let threshold_str: String = args.next().unwrap_or_else(|| "83".to_string());
    let threshold = match parse_threshold(&threshold_str) {
        Ok(n) => n,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    // Cargo-llvm-cov writes the report to `--output-path=<file>`. Use
    // a stable temp file so the path is predictable and easy to clean
    // up after extraction. A report that cannot be removed beforehand
    // could be read back as this run's result, so that is fatal.
    let report_path = std::env::temp_dir().join("slugaudit-check-coverage.cov.json");
    if let Err(err) = remove_report(&report_path) {
        eprintln!(
            "FAIL: could not remove the stale coverage report at {}: {err}",
            report_path.display()
        );
        return ExitCode::from(1);
    }

    let status = Command::new("cargo")
        .args([
            "llvm-cov",
            "--all-targets",
            "--all-features",
            "--locked",
            "--json",
        ])
        .arg("--ignore-filename-regex=.*src/bin/.*")
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
            return cleanup_then(&report_path, ExitCode::from(1));
        }
    };
    let (covered, count) = match merge_line_totals(&cov) {
        Some(v) => v,
        None => {
            eprintln!("FAIL: coverage JSON missing data[*].totals.lines");
            return cleanup_then(&report_path, ExitCode::from(1));
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
        return cleanup_then(&report_path, code);
    }
    cleanup_then(&report_path, ExitCode::SUCCESS)
}

/// Parse the gate threshold. `f64::from_str` accepts `NaN` and `inf`,
/// and every comparison against NaN is false, so a NaN gate would pass at
/// any coverage; only a finite percentage in `0..=100` is a usable gate.
fn parse_threshold(raw: &str) -> Result<f64, String> {
    let value: f64 = raw
        .parse()
        .map_err(|err| format!("threshold {raw:?} is not a number: {err}"))?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(format!(
            "threshold {raw:?} must be a finite percentage between 0 and 100"
        ));
    }
    Ok(value)
}

/// Remove the report file; an already-missing file is the expected state.
fn remove_report(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Remove the report after the verdict is known. A cleanup failure fails
/// the run (reported once) rather than being dropped; it never turns a
/// failing verdict into a pass.
fn cleanup_then(path: &std::path::Path, verdict: ExitCode) -> ExitCode {
    match remove_report(path) {
        Ok(()) => verdict,
        Err(err) => {
            eprintln!("FAIL: could not remove {}: {err}", path.display());
            ExitCode::from(1)
        }
    }
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
    fn removing_a_missing_report_succeeds_and_an_unremovable_one_fails() {
        let dir = tempfile::tempdir().expect("tempdir");

        remove_report(&dir.path().join("absent.json")).expect("missing file is fine");
        // A directory cannot be removed with remove_file.
        assert!(remove_report(dir.path()).is_err());
    }

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
