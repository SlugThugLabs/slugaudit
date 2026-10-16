//! Command-line parsing for the performance gate.

use crate::DEFAULT_THRESHOLD_PCT;

/// What one invocation asked for.
#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    Help,
    Run(Options),
}

#[derive(Debug, PartialEq)]
pub(crate) struct Options {
    /// Re-record the baseline from this run instead of comparing against it.
    pub(crate) record: bool,
    /// Allowed median regression, in percent.
    pub(crate) threshold_pct: f64,
}

/// Parses `[--record] [--help] [threshold_percent]`; an `Err` is a usage
/// error the caller reports with exit code 2.
pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut options = Options {
        record: false,
        threshold_pct: DEFAULT_THRESHOLD_PCT,
    };
    for arg in args {
        match arg.as_str() {
            "--record" => options.record = true,
            "--help" => return Ok(Command::Help),
            other => {
                options.threshold_pct = other
                    .parse::<f64>()
                    .map_err(|err| format!("threshold {other:?} is not a number: {err}"))?;
            }
        }
    }
    Ok(Command::Run(options))
}
