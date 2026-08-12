//! Production Rust code-line limit checker.
//!
//! Replaces `tools/check_source_limits.sh`. Counts the number of lines
//! containing Rust tokens *outside* comments, strings, char literals,
//! and raw-string literals per file under `src/`. Files at 0–199 such
//! lines auto-pass; 200–300 requires a `slugaudit-line-exception:`
//! justification comment with `approved-by=agent; reason=...`; ≥300 is
//! a hard failure. Output format is intentionally identical to the
//! prior shell script so existing log scrapers and CI greps keep
//! working.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. The state
//! machine lives in the sibling [`counter`] module and the unit tests
//! in the sibling [`tests`] module.

mod counter;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;

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
        match (n, counter::exception_reason(&source)) {
            (n, _) if n > 300 => {
                failures.push(format!("{rel}: {n} code lines (>300; hard failure)"));
            }
            (n, None) if n >= 200 => {
                failures.push(format!(
                    "{rel}: {n} code lines (200-300 requires an exception)"
                ));
            }
            (n, Some(reason)) if n >= 200 => {
                println!("source-limit: {rel}: {n} lines; exception: {reason}");
            }
            (n, _) => {
                println!("source-limit: {rel}: {n} code lines; pass");
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
