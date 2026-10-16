//! Comparing this run's medians against the baseline. Pure: no I/O, so
//! every verdict rule is unit-testable.

use std::collections::BTreeMap;
use std::fmt;

use crate::estimates::BaselineEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Measured but absent from the baseline.
    New,
    Ok,
    /// Slower than the baseline by more than the threshold.
    Regressed,
    /// Over the bench's absolute `budget_ns`; outranks `Regressed`.
    OverBudget,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::New => "new",
            Self::Ok => "ok",
            Self::Regressed => "REGRESSED",
            Self::OverBudget => "OVER BUDGET",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub(crate) key: String,
    /// `(baseline median, new/baseline ratio)`; `None` for a new bench.
    pub(crate) baseline: Option<(u64, f64)>,
    pub(crate) new_ns: u64,
    pub(crate) verdict: Verdict,
}

/// Everything the gate reports, in print order.
#[derive(Debug, Default)]
pub(crate) struct Comparison {
    /// One per measured bench, sorted by key.
    pub(crate) rows: Vec<Row>,
    pub(crate) warnings: Vec<String>,
    pub(crate) failures: Vec<String>,
}

pub(crate) fn compare(
    measured: &BTreeMap<String, u64>,
    baseline: &BTreeMap<String, BaselineEntry>,
    threshold_pct: f64,
) -> Comparison {
    let mut out = Comparison::default();
    for key in measured.keys().filter(|key| !baseline.contains_key(*key)) {
        out.warnings.push(format!(
            "bench {key} is not in the baseline -- run 'check_performance --record' to add it"
        ));
    }
    for key in baseline.keys().filter(|key| !measured.contains_key(*key)) {
        out.failures.push(format!(
            "{key}: produced no measurements this run (did it skip?) -- the gate cannot check a bench that did not run"
        ));
    }
    // `measured` is a BTreeMap, so rows come out already sorted by key.
    for (key, &new_ns) in measured {
        let row = match baseline.get(key) {
            None => Row {
                key: key.clone(),
                baseline: None,
                new_ns,
                verdict: Verdict::New,
            },
            Some(entry) => judge(key, new_ns, entry, threshold_pct, &mut out.failures),
        };
        out.rows.push(row);
    }
    out
}

/// One baselined bench's row; pushes a failure line per broken rule.
fn judge(
    key: &str,
    new_ns: u64,
    entry: &BaselineEntry,
    threshold_pct: f64,
    failures: &mut Vec<String>,
) -> Row {
    let ratio = if entry.median_ns == 0 {
        f64::INFINITY
    } else {
        new_ns as f64 / entry.median_ns as f64
    };
    let mut verdict = Verdict::Ok;
    if ratio > 1.0 + threshold_pct / 100.0 {
        verdict = Verdict::Regressed;
        failures.push(format!(
            "{key}: {:.1}% slower than baseline",
            (ratio - 1.0) * 100.0
        ));
    }
    if let Some(budget) = entry.budget_ns
        && new_ns > budget
    {
        verdict = Verdict::OverBudget;
        failures.push(format!(
            "{key}: {:.1} ms exceeds the {:.0} ms budget",
            new_ns as f64 / 1e6,
            budget as f64 / 1e6
        ));
    }
    Row {
        key: key.to_string(),
        baseline: Some((entry.median_ns, ratio)),
        new_ns,
        verdict,
    }
}
