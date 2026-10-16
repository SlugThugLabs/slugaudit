#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! Performance regression gate.
//!
//! Replaces `tools/check_performance.sh`. Runs the discovery, parsing,
//! and search criterion benches with a reduced sample size (fast enough
//! for CI), parses
//! criterion's median point estimates, and fails when any bench
//! regresses more than the threshold against the committed baseline
//! (`.planning/archive/perf_baseline.json` — the planning-record archive
//! is where the committed baseline lives since commit 231e8c8, even
//! though the gate consumes it as live state). The same policy as the
//! coverage gate: the gated numbers are printed explicitly, so there is
//! never a silent failure.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. `cargo
//! bench` is invoked through `std::process::Command`; estimate JSONs
//! are parsed with `serde_json` (already a dependency).
//!
//! Module split: `args` (CLI), `bench` (produce measurements), `estimates`
//! (read criterion output and the baseline), `compare` (pure verdicts),
//! `record` (rewrite the baseline), `format` (print).
//!
//! Usage: `cargo run --bin check_performance --locked [--record] [threshold-percent]`
//!   --record: regenerate `.planning/archive/perf_baseline.json` from this run.
//!   threshold: regression threshold percent (default 20).

mod args;
mod bench;
mod compare;
mod estimates;
mod format;
mod record;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::Path;
use std::process::ExitCode;

use args::{Command, Options};

const BASELINE_PATH: &str = ".planning/archive/perf_baseline.json";
const CRITERION_DIR: &str = "target/criterion";
const DEFAULT_THRESHOLD_PCT: f64 = 20.0;

fn main() -> ExitCode {
    let options = match args::parse(
        std::env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned()),
    ) {
        Ok(Command::Run(options)) => options,
        Ok(Command::Help) => {
            println!(
                "Performance regression gate. Usage: check_performance [--record] [threshold_percent]"
            );
            return ExitCode::SUCCESS;
        }
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("FAIL: {err}");
            ExitCode::from(1)
        }
    }
}

/// Measures, then either records the baseline or gates against it.
fn run(options: &Options) -> Result<ExitCode, String> {
    let baseline_path = Path::new(BASELINE_PATH);
    if !options.record && !baseline_path.exists() {
        return Err(format!(
            "no baseline at {BASELINE_PATH}; run 'cargo run --bin check_performance -- --record' first"
        ));
    }
    bench::run_fresh()?;
    let measured = estimates::collect_new_benches().map_err(|err| err.to_string())?;
    if measured.is_empty() {
        return Err(format!(
            "no criterion estimates found under {CRITERION_DIR}"
        ));
    }
    let baseline = if baseline_path.exists() {
        estimates::load_baseline(baseline_path)?
    } else {
        Default::default()
    };

    if options.record {
        let machine = format!(
            "current run (re-recorded {})",
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
        let json = record::baseline_json(&measured, &baseline, options.threshold_pct, &machine);
        record::write(baseline_path, &json)?;
        println!(
            "recorded {} benchmarks to {}",
            measured.len(),
            baseline_path.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let comparison = compare::compare(&measured, &baseline, options.threshold_pct);
    format::print_report(&comparison, options.threshold_pct, baseline_path);
    if !comparison.failures.is_empty() {
        return Ok(ExitCode::from(1));
    }
    println!(
        "PASS: no benchmark regressed more than {:.0}%",
        options.threshold_pct
    );
    Ok(ExitCode::SUCCESS)
}
