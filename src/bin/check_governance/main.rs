#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! Governance gate.
//!
//! The governance shell (`ec10812`, 2026-09-30) is a fixed set of files, and
//! `CODEOWNERS` names paths that must resolve. This gate holds both down in
//! the same spirit as [`check_docs_drift`](../check_docs_drift/main.rs): a
//! missing required file and a `CODEOWNERS` pattern pointing at nothing are
//! both machine-checkable drift.
//!
//! 1. **Required files** — every file in the fixed governance set exists
//!    ([`required_files`]).
//! 2. **`CODEOWNERS` paths** — every literal (non-glob) pattern in
//!    `.github/CODEOWNERS` references a path that exists ([`codeowners`]).
//!
//! Pure-Rust: no Python, no shell. stdlib only (JSON is parsed textually
//! here; the shape is fixed and small).

mod codeowners;
#[path = "../gate_common/args.rs"]
mod gate_args;
mod required_files;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::process::ExitCode;

use codeowners::check_codeowners;
use required_files::check_required_files;

fn main() -> ExitCode {
    let root = match gate_args::parse(
        "governance",
        "Checks that the governance shell files exist and that CODEOWNERS references \
                     only paths that exist.",
        &mut [],
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };

    let mut failures: Vec<String> = Vec::new();

    // Existence first: a missing file is reported before the CODEOWNERS
    // check, whose own verdict on an absent file would be vacuous.
    check_required_files(&root, &mut failures);
    check_codeowners(&root, &mut failures);

    for failure in &failures {
        println!("governance: {failure}");
    }
    if !failures.is_empty() {
        println!("governance: FAIL");
        return ExitCode::from(1);
    }
    println!("governance: PASS");
    ExitCode::SUCCESS
}
