//! Production Rust code-line limit checker.
#![forbid(unsafe_code)]

//!
//! Replaces `tools/check_source_limits.sh`. Counts the number of lines
//! containing Rust tokens *outside* comments, strings, char literals,
//! and raw-string literals per file under `src/`. Production files at
//! 0–199 lines auto-pass; 200–300 requires a `slugaudit-line-exception:`
//! justification comment with `approved-by=agent; reason=...`; ≥300 is a
//! hard failure. Test files (`*_tests.rs`, `tests.rs`) enumerate one
//! named behavior per `#[test]` — repetition that cannot be DRY'd without
//! losing failure diagnostics — so they auto-pass up to 500 lines; >500
//! is a hard failure with no exception path. Output format is
//! intentionally identical to the prior shell script so existing log
//! scrapers and CI greps keep working.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. The state
//! machine lives in the sibling [`counter`] module and the unit tests
//! in the sibling [`tests`] module.

mod approval;
mod counter;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A file is a test file when its name follows this repo's test-module
/// convention (`query_tests.rs`, `manager_tests.rs`, …) or is the
/// `tests.rs` shorthand used by the checker's own test module. Test
/// modules are `#[path]`-included siblings, so the name is a reliable
/// classifier.
fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("_tests.rs") || name == "tests.rs")
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut project_root = std::env::current_dir().expect("cwd");

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!("Checks production Rust code-line limits under src/.");
                return ExitCode::SUCCESS;
            }
            "--root" => {
                let Some(value) = args.next() else {
                    eprintln!("--root requires a project directory");
                    return ExitCode::from(2);
                };
                project_root = PathBuf::from(value);
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let src_root = project_root.join("src");
    if !src_root.is_dir() {
        println!("source-limit: no production Rust files found under src/");
        return ExitCode::SUCCESS;
    }

    let files = counter::walk_rs_files(&src_root);
    if files.is_empty() {
        println!("source-limit: no production Rust files found under src/");
        return ExitCode::SUCCESS;
    }

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let n = counter::code_lines(&source);
        let rel = path
            .strip_prefix(&project_root)
            .unwrap_or(path)
            .display()
            .to_string();
        match counter::verdict(n, approval::reason(&source), is_test_file(path)) {
            counter::Verdict::Pass => {
                println!("source-limit: {rel}: {n} code lines; pass");
            }
            counter::Verdict::PassWithException { reason } => {
                println!("source-limit: {rel}: {n} lines; exception: {reason}");
            }
            counter::Verdict::FailHard { ceiling } => {
                failures.push(format!("{rel}: {n} code lines (>{ceiling}; hard failure)"));
            }
            counter::Verdict::FailNeedsException => {
                failures.push(format!(
                    "{rel}: {n} code lines (>= {} requires an approved exception)",
                    counter::PRODUCTION_EXCEPTION_FLOOR
                ));
            }
        }
    }

    if !failures.is_empty() {
        println!("source-limit: FAIL");
        for failure in &failures {
            println!("  {failure}");
        }
        return ExitCode::from(1);
    }
    println!("source-limit: PASS");
    ExitCode::SUCCESS
}
