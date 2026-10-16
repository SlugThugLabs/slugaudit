#![forbid(unsafe_code)]

//! Process-level tests for the development gate binaries. Their logic is
//! unit-tested in place; these pin what CI actually consumes — the exit
//! code — so a gate whose verdict never reaches its status cannot pass a
//! broken tree, and a gate pointed at the wrong directory cannot pass at all.

use std::path::Path;
use std::process::{Command, Output};

/// Every gate that takes `--root`, with its built executable.
const ROOTED_GATES: &[(&str, &str)] = &[
    (
        "check_source_limits",
        env!("CARGO_BIN_EXE_check_source_limits"),
    ),
    (
        "check_no_duplicates",
        env!("CARGO_BIN_EXE_check_no_duplicates"),
    ),
    ("check_docs_drift", env!("CARGO_BIN_EXE_check_docs_drift")),
    (
        "check_producer_coverage",
        env!("CARGO_BIN_EXE_check_producer_coverage"),
    ),
    ("check_mcp_surface", env!("CARGO_BIN_EXE_check_mcp_surface")),
    ("check_governance", env!("CARGO_BIN_EXE_check_governance")),
    (
        "check_version_consistency",
        env!("CARGO_BIN_EXE_check_version_consistency"),
    ),
];

/// The gates without `--root`; they still own a usage contract.
const UNROOTED_GATES: &[(&str, &str)] = &[
    ("check_coverage", env!("CARGO_BIN_EXE_check_coverage")),
    ("check_performance", env!("CARGO_BIN_EXE_check_performance")),
];

const USAGE_EXIT: i32 = 2;

fn run(binary: &str, args: &[&str], cwd: &Path) -> Output {
    Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn gate binary")
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, content).expect("write fixture");
}

#[test]
fn every_gate_rejects_an_unknown_argument_as_a_usage_error() {
    let cwd = tempfile::tempdir().expect("temp dir");

    for (name, binary) in ROOTED_GATES.iter().chain(UNROOTED_GATES) {
        let output = run(binary, &["--no-such-flag"], cwd.path());

        assert_eq!(
            output.status.code(),
            Some(USAGE_EXIT),
            "{name} must reject an unknown argument, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn every_rooted_gate_fails_closed_on_a_root_with_nothing_to_check() {
    let empty = tempfile::tempdir().expect("temp dir");
    let root = empty.path().to_str().expect("utf-8 temp path");

    for (name, binary) in ROOTED_GATES {
        let output = run(binary, &["--root", root], empty.path());

        assert!(
            !output.status.success(),
            "{name} must not pass an empty root, stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn the_source_limit_gate_passes_a_small_file_and_fails_an_oversized_one() {
    let project = tempfile::tempdir().expect("temp dir");
    let root = project.path().to_str().expect("utf-8 temp path");
    let gate = env!("CARGO_BIN_EXE_check_source_limits");
    write(project.path(), "src/lib.rs", "pub fn small() {}\n");

    let small = run(gate, &["--root", root], project.path());
    let body: String = (0..360).map(|i| format!("pub fn f{i}() {{}}\n")).collect();
    write(project.path(), "src/big.rs", &body);
    let big = run(gate, &["--root", root], project.path());

    assert!(
        small.status.success(),
        "a 1-line file must pass, stdout: {}",
        String::from_utf8_lossy(&small.stdout)
    );
    assert_eq!(
        big.status.code(),
        Some(1),
        "a file past 350 code lines must fail"
    );
    assert!(
        String::from_utf8_lossy(&big.stdout).contains("big.rs"),
        "the failure must name the file, stdout: {}",
        String::from_utf8_lossy(&big.stdout)
    );
}
