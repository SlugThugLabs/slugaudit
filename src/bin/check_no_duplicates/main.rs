#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! No-duplicates gate.
//!
//! Replaces `tools/check_no_duplicates.sh`. Fails when:
//! - a commit subject occurs more than once in reachable history (the
//!   fingerprint of the same logical change being committed twice), or
//! - a `#[test]`-attributed function name is defined in more than one
//!   file under `src/` or `tests/`.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. `git log` is
//! invoked through `std::process::Command`; the test-name scan reads
//! each Rust file with stdlib only.

mod commit_subjects;
#[path = "../gate_common/args.rs"]
mod gate_args;
mod test_names;
use std::process::ExitCode;

fn main() -> ExitCode {
    let project_root = match gate_args::parse(
        "no-duplicates",
        "Checks for duplicated commit subjects and duplicated test names.",
        &mut [],
    ) {
        Ok(root) => root,
        Err(code) => return code,
    };

    let mut failures: Vec<String> = Vec::new();

    for (subject, count) in commit_subjects::duplicate_commit_subjects(&project_root, &mut failures)
    {
        failures.push(format!("commit subject {subject:?} used {count} times"));
        println!("no-duplicates: commit subject {subject:?} used {count} times");
    }

    let test_files = test_names::collect_test_files(&project_root);
    let names = test_names::test_names_by_file(&test_files);
    let mut sorted_names: Vec<_> = names.iter().filter(|(_, files)| files.len() > 1).collect();
    sorted_names.sort_by_key(|(name, _)| *name);
    for (name, files) in sorted_names {
        let joined = files
            .iter()
            .map(|p| {
                p.strip_prefix(&project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ");
        let msg = format!("test {name:?} defined in {} files: {joined}", files.len());
        println!("no-duplicates: {msg}");
        failures.push(msg);
    }

    if !failures.is_empty() {
        println!("no-duplicates: FAIL");
        return ExitCode::from(1);
    }
    println!("no-duplicates: PASS");
    ExitCode::SUCCESS
}
