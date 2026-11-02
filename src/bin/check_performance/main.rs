#![forbid(unsafe_code)]
// Panicking indexing is denied outside tests, as in the library root
// (`lib.rs`); `unwrap` and `expect` are denied for every target in
// `Cargo.toml`.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

//! Performance regression gate.
//!
//! Runs the discovery, parsing,
//! and search criterion benches with a reduced sample size (fast enough
//! for CI), parses
//! criterion's median point estimates, and fails when any bench
//! regresses more than the threshold against the committed baseline
//! (`.planning/archive/perf_baseline.json` — the planning-record archive
//! is where the committed baseline lives, even though the gate consumes
//! it as live state). The same policy as the
//! coverage gate: the gated numbers are printed explicitly, so there is
//! never a silent failure.
//!
//! Pure Rust: no Python interpreter, no shell. `cargo
//! bench` is invoked through `std::process::Command`; estimate JSONs
//! are parsed with `serde_json` (already a dependency).
//!
//! Module split: `args` (CLI), `bench` (produce measurements), `estimates`
//! (read criterion output and the baseline), `compare` (pure verdicts),
//! `record` (rewrite the baseline), `format` (print).
//!
//! Usage: `cargo run --bin check_performance --locked [--root <dir>] [--record] [threshold-percent]`
//!   --root: the project to gate (default: the working directory); the
//!     baseline is read from it and `cargo bench` runs in it.
//!   --record: regenerate `.planning/archive/perf_baseline.json` from this run.
//!     Fails when a baselined bench produced no measurement, so a record never
//!     drops an entry or its hand-set `budget_ns`.
//!   threshold: regression threshold percent (default 20).
//!
//! Criterion's output is read from `$CARGO_TARGET_DIR/criterion` when that
//! variable is set, else `target/criterion`.

mod args;
mod bench;
mod compare;
mod estimates;
mod format;
#[path = "../gate_common/args.rs"]
mod gate_args;
#[path = "../gate_common/out.rs"]
mod gate_out;
mod record;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::Path;
use std::process::ExitCode;

use args::Options;
use compare::Comparison;

const PREFIX: &str = "performance";
const HELP: &str =
    "Performance regression gate. Usage: check_performance [--record] [threshold_percent]";
const BASELINE_PATH: &str = ".planning/archive/perf_baseline.json";
const DEFAULT_THRESHOLD_PCT: f64 = 20.0;

/// What a run produced, before anything is printed.
enum Outcome {
    /// The baseline was rewritten from this many measured benches.
    Recorded(usize),
    /// This run's medians, compared against the baseline.
    Compared(Comparison),
}

fn main() -> ExitCode {
    let mut record = false;
    let mut positionals: Vec<String> = Vec::new();
    let root = match gate_args::parse(
        PREFIX,
        HELP,
        &mut [("--record", &mut record)],
        Some(&mut positionals),
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };
    let options = match args::options(record, &positionals) {
        Ok(options) => options,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    match run(&options, &root) {
        Ok(outcome) => match print_outcome(&outcome, options.threshold_pct) {
            Ok(code) => code,
            Err(error) => gate_out::failed(PREFIX, &error),
        },
        Err(err) => {
            eprintln!("FAIL: {err}");
            ExitCode::from(1)
        }
    }
}

/// Measures in `root`, then either records the baseline or gates against it.
fn run(options: &Options, root: &Path) -> Result<Outcome, String> {
    let baseline_path = root.join(BASELINE_PATH);
    if !options.record && !baseline_path.exists() {
        return Err(format!(
            "no baseline at {BASELINE_PATH}; run 'cargo run --bin check_performance -- --record' first"
        ));
    }
    let target_dir = std::env::var_os("CARGO_TARGET_DIR");
    let criterion_dir = estimates::criterion_dir(root, target_dir.as_deref());
    bench::run_fresh(root, &criterion_dir)?;
    let measured = estimates::collect_new_benches(&criterion_dir).map_err(|err| err.to_string())?;
    if measured.is_empty() {
        return Err(format!(
            "no criterion estimates found under {}",
            criterion_dir.display()
        ));
    }
    let baseline = if baseline_path.exists() {
        estimates::load_baseline(&baseline_path)?
    } else {
        std::collections::BTreeMap::default()
    };

    if options.record {
        let missing = record::unmeasured(&measured, &baseline);
        if !missing.is_empty() {
            return Err(format!(
                "cannot record: baselined benches produced no measurements: {}; \
                 remove them from {BASELINE_PATH} by hand if they no longer exist",
                missing.join(", ")
            ));
        }
        // No cwd is recorded: it would embed a developer's absolute path in a committed file.
        record::write(&baseline_path, &record::baseline_json(&measured, &baseline))?;
        return Ok(Outcome::Recorded(measured.len()));
    }

    Ok(Outcome::Compared(compare::compare(
        &measured,
        &baseline,
        options.threshold_pct,
    )))
}

/// Prints the run's report; the exit code is 1 when a bench failed.
fn print_outcome(outcome: &Outcome, threshold_pct: f64) -> std::io::Result<ExitCode> {
    let baseline_path = Path::new(BASELINE_PATH);
    match outcome {
        Outcome::Recorded(count) => {
            gate_out::line(format_args!(
                "recorded {count} benchmarks to {}",
                baseline_path.display()
            ))?;
            Ok(ExitCode::SUCCESS)
        }
        Outcome::Compared(comparison) => {
            format::print_report(comparison, threshold_pct, baseline_path)?;
            if !comparison.failures.is_empty() {
                return Ok(ExitCode::from(1));
            }
            gate_out::line(format_args!(
                "PASS: no benchmark regressed more than {threshold_pct:.0}%"
            ))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
