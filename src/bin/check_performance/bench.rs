//! Producing this run's criterion measurements.

use crate::CRITERION_DIR;

/// The gated benches and the reduced-sample criterion settings that keep a
/// run fast enough to repeat before and after a change.
const BENCH_ARGS: &[&str] = &[
    "bench",
    "--locked",
    "--bench",
    "discovery",
    "--bench",
    "parsing",
    "--bench",
    "search",
    "--",
    "--sample-size",
    "15",
    "--warm-up-time",
    "0.5",
    "--measurement-time",
    "3",
];

/// Clears stale criterion output, then runs the gated benches.
///
/// Never trust the previous run's numbers: a failed cleanup must stop the
/// run, because a leftover `new/estimates.json` would otherwise be harvested
/// as this run's measurement.
pub(crate) fn run_fresh() -> Result<(), String> {
    match std::fs::remove_dir_all(CRITERION_DIR) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "could not clear stale criterion output {CRITERION_DIR}: {err}"
            ));
        }
    }
    let status = std::process::Command::new("cargo")
        .args(BENCH_ARGS)
        .status()
        .map_err(|err| format!("failed to invoke cargo bench: {err}"))?;
    if !status.success() {
        return Err(format!("cargo bench did not complete: {status}"));
    }
    Ok(())
}
