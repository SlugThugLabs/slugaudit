#![forbid(unsafe_code)]
// Panicking indexing is denied outside tests, as in the library root
// (`lib.rs`); `unwrap` and `expect` are denied for every target in
// `Cargo.toml`.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

//! Line coverage gate.
//!
//! Reads the merged JSON line
//! coverage — the same metric `cargo llvm-cov --fail-under-lines`
//! compares (`totals.lines.percent`) — and fails the run if it is
//! below the threshold. The measured number is printed so the gated
//! value is never ambiguous: the `--summary-only` text table prints
//! region-weighted columns that are easily misread as line coverage,
//! and that printed table is not what the exit check compares
//! against.
//!
//! Scope: the gate measures the shipped product runtime. The dev-only
//! `src/bin/check_*` gate binaries are excluded via
//! `--ignore-filename-regex`: a coverage audit must not count
//! development-only check binaries as end-user runtime code, and
//! measuring them here while their own test files are excluded from the
//! report would leave six gate mains permanently at 0% inside the
//! denominator (~8 points of measured coverage). The floor is measured
//! product coverage minus a margin.
//!
//! Pure Rust: no Python interpreter, no shell. `cargo
//! llvm-cov` is invoked through `std::process::Command`; the JSON
//! report is parsed with `serde_json` (already a dependency).

#[path = "gate_common/args.rs"]
mod gate_args;
#[path = "gate_common/out.rs"]
mod gate_out;

use std::path::Path;
use std::process::{Command, ExitCode};

const PREFIX: &str = "coverage";

/// The line-coverage floor, in percent, when no threshold argument is given.
/// It tracks the measured line coverage. The gate's intent is "catch coverage
/// regressions, not block on them", so it tracks measurement rather than an
/// aspirational number. Use `cargo run --quiet --bin check_coverage --
/// <threshold>` to override for a single run (e.g. CI flavor that wants a
/// harder floor); record any raise with the change that makes it so the gate
/// stays honest.
const DEFAULT_THRESHOLD_PCT: f64 = 83.0;

fn main() -> ExitCode {
    let mut positionals: Vec<String> = Vec::new();
    let root = match gate_args::parse(
        PREFIX,
        &format!(
            "Line coverage gate. Usage: check_coverage [--root <dir>] [threshold_percent] \
             (default {DEFAULT_THRESHOLD_PCT})"
        ),
        &mut [],
        Some(&mut positionals),
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };
    let threshold = match positionals.as_slice() {
        [] => DEFAULT_THRESHOLD_PCT,
        [raw] => match parse_threshold(raw) {
            Ok(value) => value,
            Err(message) => {
                eprintln!("{message}");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("expected at most one threshold argument, got {positionals:?}");
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

    // Every exit from here on has run (or tried to run) cargo, so every
    // exit removes the report exactly once.
    match measure(&root, &report_path) {
        Err(message) => {
            eprintln!("{message}");
            cleanup_then(&report_path, ExitCode::from(1))
        }
        Ok((covered, count)) => match gate(covered, count, threshold) {
            Err(message) => {
                eprintln!("{message}");
                cleanup_then(&report_path, ExitCode::from(1))
            }
            Ok((percent, passed)) => {
                let verdict = match print_verdict(percent, (covered, count), threshold, passed) {
                    Ok(code) => code,
                    Err(error) => gate_out::failed(PREFIX, &error),
                };
                cleanup_then(&report_path, verdict)
            }
        },
    }
}

/// Prints the measured line coverage and, below the threshold, the failure.
fn print_verdict(
    percent: f64,
    (covered, count): (u64, u64),
    threshold: f64,
    passed: bool,
) -> std::io::Result<ExitCode> {
    gate_out::line(format_args!(
        "line coverage: {percent:.2}% ({covered} / {count} lines, gate {threshold}%)"
    ))?;
    if passed {
        return Ok(ExitCode::SUCCESS);
    }
    gate_out::line(format_args!(
        "FAIL: line coverage {percent:.2}% is below the {threshold}% gate"
    ))?;
    Ok(ExitCode::from(1))
}

/// Run `cargo llvm-cov` in `root`, read the JSON report it writes to
/// `report_path`, and return the merged `(covered, count)` line totals. The
/// error is the complete `FAIL: ...` message to print.
fn measure(root: &Path, report_path: &Path) -> Result<(u64, u64), String> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "llvm-cov",
            "--all-targets",
            "--all-features",
            "--locked",
            "--json",
        ])
        .arg("--ignore-filename-regex=.*src/bin/.*")
        .arg(format!("--output-path={}", report_path.display()))
        .status()
        .map_err(|err| format!("FAIL: failed to invoke cargo llvm-cov: {err}"))?;
    if !status.success() {
        return Err(format!(
            "FAIL: cargo llvm-cov --json did not complete: {status}"
        ));
    }
    let raw = std::fs::read_to_string(report_path).map_err(|err| {
        format!(
            "FAIL: coverage report at {} was not written: {err}",
            report_path.display()
        )
    })?;
    let cov: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|err| format!("FAIL: could not parse coverage JSON: {err}"))?;
    merge_line_totals(&cov)
}

/// The line-coverage percent (0.0 when nothing was counted) and whether
/// it meets the threshold. The error is the complete `FAIL: ...` message to
/// print when a line total does not fit the `u32` the percentage is
/// computed from (no integer-to-float cast can represent it exactly).
fn gate(covered: u64, count: u64, threshold: f64) -> Result<(f64, bool), String> {
    let exact = |name: &str, total: u64| {
        u32::try_from(total).map(f64::from).map_err(|err| {
            format!("FAIL: {name} line total {total} is too large to compute a percentage: {err}")
        })
    };
    let percent = if count == 0 {
        0.0
    } else {
        exact("covered", covered)? / exact("counted", count)? * 100.0
    };
    Ok((percent, percent >= threshold))
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
fn remove_report(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Remove the report after the verdict is known. A cleanup failure fails
/// the run (reported once) rather than being dropped; it never turns a
/// failing verdict into a pass.
fn cleanup_then(path: &Path, verdict: ExitCode) -> ExitCode {
    match remove_report(path) {
        Ok(()) => verdict,
        Err(err) => {
            eprintln!("FAIL: could not remove {}: {err}", path.display());
            ExitCode::from(1)
        }
    }
}

/// The `(covered, count)` line totals of the report's single merged data
/// entry. `cargo llvm-cov --json` merges every test binary into one entry;
/// summing several would count a file shared by two entries twice, so any
/// other entry count is an error. The error is the complete `FAIL: ...`
/// message to print.
fn merge_line_totals(cov: &serde_json::Value) -> Result<(u64, u64), String> {
    let missing = || "FAIL: coverage JSON missing data[*].totals.lines".to_string();
    let data = cov
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(missing)?;
    let [entry] = data.as_slice() else {
        return Err(format!(
            "FAIL: coverage JSON has {} data entries, expected one merged entry; summing \
             them could count shared files twice",
            data.len()
        ));
    };
    let lines = entry
        .get("totals")
        .and_then(|totals| totals.get("lines"))
        .ok_or_else(missing)?;
    let total = |key: &str| {
        lines
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(missing)
    };
    Ok((total("covered")?, total("count")?))
}

#[cfg(test)]
#[path = "check_coverage/tests.rs"]
mod tests;
