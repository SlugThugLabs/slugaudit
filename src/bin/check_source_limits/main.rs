//! Production Rust code-line limit checker.
#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//!
//! Replaces `tools/check_source_limits.sh`. Counts the number of lines
//! containing Rust tokens *outside* comments, strings, char literals,
//! and raw-string literals per file under `src/`. Production files at
//! 0–199 lines auto-pass; 200–300 requires a `slugaudit-line-exception:`
//! justification comment with `approved-by=slugthug; reason=...`; ≥300 is a
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
#[path = "../gate_common/args.rs"]
mod gate_args;
mod scanner;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::Path;
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
    let project_root = match gate_args::parse(
        "source-limit",
        "Checks production Rust code-line limits under src/.",
        &mut [],
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };

    let src_root = project_root.join("src");
    // Fail closed: a root with nothing to measure is a mistyped `--root`
    // or a wrong working directory, never a project that passes.
    if !src_root.is_dir() {
        eprintln!("source-limit: {} is not a directory", src_root.display());
        return ExitCode::from(2);
    }

    let files = match counter::walk_rs_files(&src_root) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("source-limit: cannot scan {}: {err}", src_root.display());
            return ExitCode::from(1);
        }
    };
    if files.is_empty() {
        eprintln!(
            "source-limit: no production Rust files found under {}",
            src_root.display()
        );
        return ExitCode::from(2);
    }

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&project_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let source = match std::fs::read_to_string(path) {
            Ok(src) => src,
            Err(err) => {
                failures.push(format!("{rel}: read error: {err}"));
                continue;
            }
        };
        let n = counter::code_lines(&source);
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
