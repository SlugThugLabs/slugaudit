//! Gate options for the performance gate, built from what the shared
//! argument contract (`gate_common/args.rs`) collected.

use crate::DEFAULT_THRESHOLD_PCT;

#[derive(Debug, PartialEq)]
pub(crate) struct Options {
    /// Re-record the baseline from this run instead of comparing against it.
    pub(crate) record: bool,
    /// Allowed median regression, in percent.
    pub(crate) threshold_pct: f64,
}

/// Builds the options from the `--record` switch and the positional
/// arguments (`[threshold_percent]`); an `Err` is a usage error the caller
/// reports with exit code 2.
pub(crate) fn options(record: bool, positionals: &[String]) -> Result<Options, String> {
    let threshold_pct = match positionals {
        [] => DEFAULT_THRESHOLD_PCT,
        [raw] => parse_threshold(raw)?,
        _ => {
            return Err(format!(
                "expected at most one threshold, got {positionals:?}"
            ));
        }
    };
    Ok(Options {
        record,
        threshold_pct,
    })
}

fn parse_threshold(raw: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|err| format!("threshold {raw:?} is not a number: {err}"))?;
    // NaN makes every regression comparison false, so the gate
    // would pass at any slowdown; infinity and negatives are
    // equally meaningless as a regression allowance.
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "threshold {raw:?} must be a finite, non-negative percentage"
        ));
    }
    Ok(value)
}
