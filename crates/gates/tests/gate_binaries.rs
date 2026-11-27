#![forbid(unsafe_code)]

//! Process-level tests for the development gate binaries. Their logic is
//! unit-tested in place; these pin what CI actually consumes — the exit
//! code — so a gate whose verdict never reaches its status cannot pass a
//! broken tree, and a gate pointed at the wrong directory cannot pass at all.

use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Output};

/// Every gate binary takes `--root <dir>` via `gate_common/args.rs` and fails
/// closed when pointed at an empty directory.
const ROOTED_GATES: &[(&str, &str)] = &[
    (
        "check_source_limits",
        env!("CARGO_BIN_EXE_check_source_limits"),
    ),
    (
        "check_no_duplicates",
        env!("CARGO_BIN_EXE_check_no_duplicates"),
    ),
    (
        "check_producer_coverage",
        env!("CARGO_BIN_EXE_check_producer_coverage"),
    ),
    ("check_mcp_surface", env!("CARGO_BIN_EXE_check_mcp_surface")),
];

const USAGE_EXIT: i32 = 2;

fn run(binary: &str, args: &[&str], cwd: &Path) -> TestResult<Output> {
    Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .output()
        .check("spawn gate binary")
}

fn write(root: &Path, relative: &str, content: &str) -> TestResult {
    let path = root.join(relative);
    let parent = path.parent().check("fixture path has a parent")?;
    std::fs::create_dir_all(parent).check("create fixture directory")?;
    std::fs::write(path, content).check("write fixture")
}

#[test]
fn every_gate_rejects_an_unknown_argument_as_a_usage_error() -> TestResult {
    let cwd = tempfile::tempdir().check("create temp dir")?;

    for (name, binary) in ROOTED_GATES {
        let output = run(binary, &["--no-such-flag"], cwd.path())?;

        ensure_eq(
            &output.status.code(),
            &Some(USAGE_EXIT),
            format!(
                "{name} must reject an unknown argument, stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
    }
    Ok(())
}

#[test]
fn every_rooted_gate_fails_closed_on_a_root_with_nothing_to_check() -> TestResult {
    let empty = tempfile::tempdir().check("create temp dir")?;
    let root = empty.path().to_str().check("temp path is utf-8")?;

    for (name, binary) in ROOTED_GATES {
        let output = run(binary, &["--root", root], empty.path())?;

        ensure(
            !output.status.success(),
            format!(
                "{name} must not pass an empty root, stdout: {}",
                String::from_utf8_lossy(&output.stdout)
            ),
        )?;
    }
    Ok(())
}

#[test]
fn the_source_limit_gate_enforces_the_200_line_exception_floor_with_no_hard_cap() -> TestResult {
    let project = tempfile::tempdir().check("create temp dir")?;
    let root = project.path().to_str().check("temp path is utf-8")?;
    let gate = env!("CARGO_BIN_EXE_check_source_limits");
    write(project.path(), "src/lib.rs", "pub fn small() {}\n")?;

    let small = run(gate, &["--root", root], project.path())?;
    let mut body = String::new();
    for i in 0..400 {
        writeln!(body, "pub fn f{i}() {{}}").check("write to a String")?;
    }
    write(project.path(), "src/big.rs", &body)?;
    let unapproved = run(gate, &["--root", root], project.path())?;

    let approved_body = format!(
        "// slugaudit-line-exception: approved-by=split-judge; reason=single cohesive job\n{body}"
    );
    write(project.path(), "src/big.rs", &approved_body)?;
    let approved = run(gate, &["--root", root], project.path())?;

    ensure(
        small.status.success(),
        format!(
            "a 1-line file must pass, stdout: {}",
            String::from_utf8_lossy(&small.stdout)
        ),
    )?;
    ensure_eq(
        &unapproved.status.code(),
        &Some(1),
        "a file at or above 200 code lines without an approved exception must fail",
    )?;
    ensure(
        String::from_utf8_lossy(&unapproved.stdout).contains("big.rs"),
        format!(
            "the failure must name the file, stdout: {}",
            String::from_utf8_lossy(&unapproved.stdout)
        ),
    )?;
    ensure(
        approved.status.success(),
        format!(
            "a 400-line file with a split-judge exception must pass (no hard cap), stdout: {}",
            String::from_utf8_lossy(&approved.stdout)
        ),
    )?;
    Ok(())
}
