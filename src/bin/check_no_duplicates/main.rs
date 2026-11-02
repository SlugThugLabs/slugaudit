#![forbid(unsafe_code)]
// Panicking indexing is denied outside tests, as in the library root
// (`lib.rs`); `unwrap` and `expect` are denied for every target in
// `Cargo.toml`.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

//! No-duplicates gate.
//!
//! Fails when:
//! - a commit subject occurs more than once in reachable history (the
//!   fingerprint of the same logical change being committed twice), or
//! - a `#[test]`-attributed function name is defined in more than one
//!   file under `src/` or `tests/`.
//!
//! Pure Rust: no Python interpreter, no shell. `git log` is
//! invoked through `std::process::Command`; the test-name scan reads
//! each Rust file with stdlib only.

mod commit_subjects;
#[path = "../gate_common/args.rs"]
mod gate_args;
#[path = "../gate_common/out.rs"]
mod gate_out;
mod test_names;
use std::process::ExitCode;

const PREFIX: &str = "no-duplicates";

fn main() -> ExitCode {
    let project_root = match gate_args::parse(
        PREFIX,
        "Checks for duplicated commit subjects and duplicated test names.",
        &mut [],
        None,
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };
    let failures = collect_failures(&project_root);
    match report(&failures) {
        Ok(code) => code,
        Err(error) => gate_out::failed(PREFIX, &error),
    }
}

/// Every reason the gate fails: duplicated subjects, duplicated test names,
/// and each scan step that could not run.
fn collect_failures(project_root: &std::path::Path) -> Vec<String> {
    let mut failures: Vec<String> = Vec::new();

    for (subject, count) in commit_subjects::duplicate_commit_subjects(project_root, &mut failures)
    {
        failures.push(format!("commit subject {subject:?} used {count} times"));
    }

    let test_files = test_names::collect_test_files(project_root, &mut failures);
    let names = test_names::test_names_by_file(&test_files, &mut failures);
    for (name, files) in names.iter().filter(|(_, files)| files.len() > 1) {
        let mut relative_files: Vec<String> = Vec::new();
        for p in files {
            match p.strip_prefix(project_root) {
                Ok(relative) => relative_files.push(relative.display().to_string()),
                Err(error) => failures.push(format!(
                    "{} is not under the project root {}: {error}",
                    p.display(),
                    project_root.display()
                )),
            }
        }
        let joined = relative_files.join(", ");
        failures.push(format!(
            "test {name:?} defined in {} files: {joined}",
            files.len()
        ));
    }
    failures
}

/// Prints each failure once on stderr and the verdict on stdout.
fn report(failures: &[String]) -> std::io::Result<ExitCode> {
    if !failures.is_empty() {
        for failure in failures {
            eprintln!("{PREFIX}: {failure}");
        }
        gate_out::line(format_args!("{PREFIX}: FAIL"))?;
        return Ok(ExitCode::from(1));
    }
    gate_out::line(format_args!("{PREFIX}: PASS"))?;
    Ok(ExitCode::SUCCESS)
}
