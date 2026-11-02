#![forbid(unsafe_code)]
// Panicking indexing is denied outside tests, as in the library root
// (`lib.rs`); `unwrap` and `expect` are denied for every target in
// `Cargo.toml`.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

//! Producer-coverage gate.
//!
//! An `ExtractionCompleteness` variant that is never constructed, or an
//! `EvidenceKind` variant with no producer at all, is a class of defect that
//! deleting individual dead kinds does not prevent. This
//! gate makes the class mechanical: **every `EvidenceKind` variant has at
//! least one producer, and every `ExtractionCompleteness` variant is
//! reachable.**
//!
//! Neither list is hand-kept. The variant list is parsed from each enum's own
//! declaration, and a producer is a *value-position* construction of the
//! variant in the production sources under `src/` — never a test fixture, and
//! never a match arm or comparison. That last distinction is load-bearing:
//! the SQL mirror (`src/evidence/sql.rs::kind_text`,
//! `src/model/parser.rs::ExtractionCompleteness::as_sql_text`) enumerates
//! every variant, so counting any occurrence at all would make the gate
//! vacuous. See [`scan`] for the exact rules.
//!
//! Limits, stated so they are not silently trusted. A producer written in a
//! style the scanner does not recognize (for example `let kind =
//! EvidenceKind::X;` used via the variable) reports as *uncovered*: the gate
//! fails and a human decides, never silently accepting. In the other
//! direction, comments are stripped but **string literals are not**, so a
//! variant's name inside a string in a value position would count as a
//! producer; keeping string state would mean a second scanner for a case no
//! producer in this repo writes.
//!
//! Pure-Rust: no Python, no shell. stdlib only.

#[path = "../gate_common/args.rs"]
mod gate_args;
#[path = "../gate_common/out.rs"]
mod gate_out;
#[path = "../gate_common/test_file.rs"]
mod gate_test_file;
mod scan;
mod test_cfg;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

/// An enum the gate requires producer coverage for: its name, and the file
/// that declares it.
struct GatedEnum {
    name: &'static str,
    declaration: &'static str,
}

const GATED: &[GatedEnum] = &[
    GatedEnum {
        name: "EvidenceKind",
        declaration: "src/model/evidence.rs",
    },
    GatedEnum {
        name: "ExtractionCompleteness",
        declaration: "src/model/parser.rs",
    },
];

/// One gated enum's coverage, as the gate found it.
pub(crate) struct EnumReport {
    pub(crate) name: &'static str,
    pub(crate) declared: usize,
    pub(crate) covered: usize,
    pub(crate) uncovered: Vec<String>,
}

const PREFIX: &str = "producer-coverage";

fn main() -> ExitCode {
    let root = match gate_args::parse(
        PREFIX,
        "Checks that every evidence kind and extraction-completeness variant has a \
                     producer in src/.",
        &mut [],
        None,
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };

    let reports = match check(&root) {
        Ok(reports) => reports,
        Err(error) => {
            eprintln!("{PREFIX}: {error}");
            return ExitCode::from(2);
        }
    };
    match print_verdict(&reports) {
        Ok(code) => code,
        Err(error) => gate_out::failed(PREFIX, &error),
    }
}

/// Prints each enum's coverage and the verdict; the exit code is 1 when any
/// variant has no producer.
fn print_verdict(reports: &[EnumReport]) -> std::io::Result<ExitCode> {
    let mut failures: Vec<String> = Vec::new();
    for report in reports {
        gate_out::line(format_args!(
            "{PREFIX}: {}: {}/{} variants produced",
            report.name, report.covered, report.declared
        ))?;
        for variant in &report.uncovered {
            failures.push(format!(
                "{}::{variant} has no producer in src/",
                report.name
            ));
        }
    }
    if !failures.is_empty() {
        gate_out::line(format_args!("{PREFIX}: FAIL"))?;
        for failure in &failures {
            gate_out::line(format_args!("  {failure}"))?;
        }
        return Ok(ExitCode::from(1));
    }
    gate_out::line(format_args!("{PREFIX}: PASS"))?;
    Ok(ExitCode::SUCCESS)
}

/// Computes one [`EnumReport`] per gated enum.
///
/// # Errors
///
/// Returns a message when `src/` cannot be walked, a declaration file cannot
/// be read, or an enum's declaration does not parse — a gate that cannot read
/// what it verifies must not print a verdict.
pub(crate) fn check(root: &Path) -> Result<Vec<EnumReport>, String> {
    let sources = scan::production_sources(root)?;
    if sources.is_empty() {
        return Err(format!(
            "no production Rust sources under {}",
            root.join("src").display()
        ));
    }
    let mut reports = Vec::with_capacity(GATED.len());
    for gated in GATED {
        let path = root.join(gated.declaration);
        let source = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let variants = scan::enum_variants(&source, gated.name).ok_or_else(|| {
            format!(
                "{} declares no `pub enum {}`",
                gated.declaration, gated.name
            )
        })?;
        if variants.is_empty() {
            return Err(format!(
                "{}: `pub enum {}` parsed with no variants — the parser disagrees with the source",
                gated.declaration, gated.name
            ));
        }
        let produced: BTreeSet<String> = sources
            .iter()
            .flat_map(|(_, text)| scan::producer_variants(text, gated.name))
            .collect();
        let uncovered = variants
            .iter()
            .filter(|variant| !produced.contains(*variant))
            .cloned()
            .collect::<Vec<_>>();
        reports.push(EnumReport {
            name: gated.name,
            declared: variants.len(),
            covered: variants.len() - uncovered.len(),
            uncovered,
        });
    }
    Ok(reports)
}
