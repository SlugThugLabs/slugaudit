//! Docs-drift gate.
//!
//! Replaces the never-built `tools/check_docs_drift.sh` from plan C8. Fails
//! when the documentation and the codebase disagree in a machine-checkable
//! way. The four checks live in sibling modules — [`module_map`],
//! [`test_file_count`], and [`plan`] — sharing this crate root's argv
//! parser, failure-printer, and process exit-code contract, so a single
//! gate verdict stays in one process.
//!
//! 1. **`ARCHITECTURE.md` module map** — every file/directory the module-map
//!    tree references must exist ([`module_map`]).
//! 2. **`*_tests.rs` count claim** — the prose in `ARCHITECTURE.md` ("The N
//!    `*_tests.rs` files share this pattern") must match the real number of
//!    `*_tests.rs` files under `src/` ([`test_file_count`]).
//! 3. **Plan status header freshness** — `IMPLEMENTATION_PLAN.md`'s `Status:`
//!    line must reference the §22 audit-corrections section ([`plan`]).
//! 4. **Plan tasks with no implementation** — a `### Task X.Y` section whose
//!    listed `src/` files all fail to exist must have an explicit descope
//!    entry in `DECISIONS.md` mentioning that exact task id ([`plan`]).
//!
//! Pure-Rust: no Python, no shell. stdlib only.

mod module_map;
mod plan;
mod test_file_count;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use module_map::check_module_map;
use plan::{check_plan_status_header, check_plan_task_descope};
use test_file_count::check_test_file_count;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut project_root = std::env::current_dir().expect("cwd");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!(
                    "Checks that ARCHITECTURE.md module map, test-count claims, and plan task \
                     descope entries match the codebase."
                );
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

    let mut failures: Vec<String> = Vec::new();

    check_module_map(&project_root, &mut failures);
    check_test_file_count(&project_root, &mut failures);
    check_plan_status_header(&project_root, &mut failures);
    check_plan_task_descope(&project_root, &mut failures);

    for failure in &failures {
        println!("docs-drift: {failure}");
    }
    if !failures.is_empty() {
        println!("docs-drift: FAIL");
        return ExitCode::from(1);
    }
    println!("docs-drift: PASS");
    ExitCode::SUCCESS
}

/// Reads a project file, exiting with code 2 on a read failure: a gate that
/// cannot read the document it must verify cannot produce a verdict.
fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap_or_else(|err| {
        eprintln!(
            "docs-drift: cannot read {}: {err}",
            root.join(relative).display()
        );
        std::process::exit(2);
    })
}

// Test-only wiring: shared fixtures plus per-check test modules, each kept
// under the source-size cap. `crate::test_support` holds the fixture
// helpers so no test module duplicates them (DRY).
#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "module_map_tests.rs"]
mod module_map_tests;

#[cfg(test)]
#[path = "test_file_count_tests.rs"]
mod test_file_count_tests;

#[cfg(test)]
#[path = "plan_tests.rs"]
mod plan_tests;
