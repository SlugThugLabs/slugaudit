//! Production Rust code-line limit checker.
#![forbid(unsafe_code)]
// Panicking indexing is denied outside tests, as in the library root
// (`lib.rs`); `unwrap` and `expect` are denied for every target in
// `Cargo.toml`.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

//!
//! Counts the number of lines
//! containing Rust tokens *outside* comments, strings, char literals,
//! and raw-string literals per file under `src/`. Production files at
//! 0–199 lines auto-pass; 200–300 requires a `slugaudit-line-exception:`
//! justification comment with `approved-by=slugthuglabs; reason=...`; 301–350
//! requires a dated human approval (`approved-by=human-user`); above 350 is a
//! hard failure. Test files (`*_tests.rs`, `tests.rs`) enumerate one
//! named behavior per `#[test]` — repetition that cannot be DRY'd without
//! losing failure diagnostics — so they auto-pass up to 500 lines; >500
//! is a hard failure with no exception path. Output format is
//! stable so existing log scrapers and CI greps keep working.
//!
//! Pure Rust: no Python interpreter, no shell. The state
//! machine lives in the sibling [`counter`] module and the unit tests
//! in the sibling `tests` module.

mod approval;
mod counter;
#[path = "../gate_common/args.rs"]
mod gate_args;
#[path = "../gate_common/out.rs"]
mod gate_out;
#[path = "../gate_common/test_file.rs"]
mod gate_test_file;
mod scanner;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::Path;
use std::process::ExitCode;

/// What checking one file produced: a line to print, or a failure to
/// collect for the summary.
#[derive(Debug, PartialEq, Eq)]
enum FileOutcome {
    Line(String),
    Failure(String),
}

/// Read one file, count its code lines, and turn the verdict into its
/// output line or failure line.
fn check_file(project_root: &Path, path: &Path) -> FileOutcome {
    let Ok(relative) = path.strip_prefix(project_root) else {
        return FileOutcome::Failure(format!(
            "{}: path is not under the project root {}",
            path.display(),
            project_root.display()
        ));
    };
    let rel = relative.display().to_string();
    let source = match std::fs::read_to_string(path) {
        Ok(src) => src,
        Err(err) => return FileOutcome::Failure(format!("{rel}: read error: {err}")),
    };
    let n = counter::code_lines(&source);
    match counter::verdict(
        n,
        approval::reason(&source),
        gate_test_file::is_test_file(path),
    ) {
        counter::Verdict::Pass => {
            FileOutcome::Line(format!("source-limit: {rel}: {n} code lines; pass"))
        }
        counter::Verdict::PassWithException { reason } => FileOutcome::Line(format!(
            "source-limit: {rel}: {n} lines; exception: {reason}"
        )),
        counter::Verdict::FailHard { ceiling } => {
            FileOutcome::Failure(format!("{rel}: {n} code lines (>{ceiling}; hard failure)"))
        }
        counter::Verdict::FailNeedsException => FileOutcome::Failure(format!(
            "{rel}: {n} code lines (>= {} requires an approved exception)",
            counter::PRODUCTION_EXCEPTION_FLOOR
        )),
    }
}

const PREFIX: &str = "source-limit";

fn main() -> ExitCode {
    let project_root = match gate_args::parse(
        PREFIX,
        "Checks production Rust code-line limits under src/.",
        &mut [],
        None,
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };

    let src_root = project_root.join("src");
    // Fail closed: a root with nothing to measure is a mistyped `--root`
    // or a wrong working directory, never a project that passes.
    if !src_root.is_dir() {
        eprintln!("{PREFIX}: {} is not a directory", src_root.display());
        return ExitCode::from(2);
    }

    let files = match counter::walk_rs_files(&src_root) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("{PREFIX}: cannot scan {}: {err}", src_root.display());
            return ExitCode::from(1);
        }
    };
    if files.is_empty() {
        eprintln!(
            "{PREFIX}: no production Rust files found under {}",
            src_root.display()
        );
        return ExitCode::from(2);
    }

    match print_verdict(&project_root, &files) {
        Ok(code) => code,
        Err(error) => gate_out::failed(PREFIX, &error),
    }
}

/// Prints one line per passing file and the verdict; the exit code is 1 when
/// any file fails.
fn print_verdict(project_root: &Path, files: &[std::path::PathBuf]) -> std::io::Result<ExitCode> {
    let mut failures: Vec<String> = Vec::new();
    for path in files {
        match check_file(project_root, path) {
            FileOutcome::Line(line) => gate_out::line(line)?,
            FileOutcome::Failure(failure) => failures.push(failure),
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
