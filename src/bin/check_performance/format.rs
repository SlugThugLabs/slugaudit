//! Output formatting for the performance gate.

use std::path::Path;

use crate::compare::{Comparison, Row};

pub(crate) fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2} s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.2} ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.2} us", ns as f64 / 1e3)
    } else {
        format!("{ns} ns")
    }
}

/// One table line for `row`, aligned under [`TABLE_HEADER`].
pub(crate) fn table_line(row: &Row) -> String {
    match row.baseline {
        None => format!(
            "{:<32} {:>12} {:>12} {:>7}  {}",
            row.key,
            "-",
            fmt_ns(row.new_ns),
            "-",
            row.verdict
        ),
        Some((baseline_ns, ratio)) => format!(
            "{:<32} {:>12} {:>12} {:>6.2}x  {}",
            row.key,
            fmt_ns(baseline_ns),
            fmt_ns(row.new_ns),
            ratio,
            row.verdict
        ),
    }
}

const TABLE_HEADER: [&str; 4] = ["bench", "baseline", "measured", "ratio"];

/// Prints the full comparison: header, one line per bench, then every
/// warning and failure. Nothing gated is left unprinted.
pub(crate) fn print_report(comparison: &Comparison, threshold_pct: f64, baseline_path: &Path) {
    let [bench, baseline, measured, ratio] = TABLE_HEADER;
    println!(
        "perf regression check (threshold {threshold_pct:.0}%, baseline: {})",
        baseline_path.display()
    );
    println!("{bench:<32} {baseline:>12} {measured:>12} {ratio:>7}  verdict");
    for row in &comparison.rows {
        println!("{}", table_line(row));
    }
    for warning in &comparison.warnings {
        println!("WARN: {warning}");
    }
    for failure in &comparison.failures {
        println!("FAIL: {failure}");
    }
}
