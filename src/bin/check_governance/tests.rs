//! Tests for the governance gate: the required-file set and the `CODEOWNERS`
//! path check.

use std::path::Path;

use tempfile::{TempDir, tempdir};

use super::codeowners::check_codeowners;
use super::required_files::{REQUIRED_FILES, check_required_files};

fn temp_root() -> TempDir {
    tempdir().expect("temp dir")
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directory");
    }
    std::fs::write(&path, contents).expect("write fixture");
}

fn write_governance_shell(root: &Path) {
    for file in REQUIRED_FILES {
        write(root, file, "present\n");
    }
}

#[test]
fn governance_required_files_all_present_passes() {
    let root = temp_root();
    write_governance_shell(root.path());

    let mut failures = Vec::new();
    check_required_files(root.path(), &mut failures);

    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn governance_flags_each_missing_required_file() {
    let root = temp_root();
    write_governance_shell(root.path());
    std::fs::remove_file(root.path().join("SECURITY.md")).expect("remove fixture");

    let mut failures = Vec::new();
    check_required_files(root.path(), &mut failures);

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("SECURITY.md"), "{failures:?}");
}

#[test]
fn governance_codeowners_paths_that_exist_pass() {
    let root = temp_root();
    write(root.path(), "src/lib.rs", "// fixture\n");
    write(root.path(), "README.md", "# fixture\n");
    write(
        root.path(),
        ".github/CODEOWNERS",
        "# comment\n*  @owner\n/src/  @owner\n/README.md  @owner\n",
    );

    let mut failures = Vec::new();
    check_codeowners(root.path(), &mut failures);

    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn governance_codeowners_flags_a_missing_path() {
    let root = temp_root();
    write(root.path(), ".github/CODEOWNERS", "/src/  @owner\n");

    let mut failures = Vec::new();
    check_codeowners(root.path(), &mut failures);

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("/src/"), "{failures:?}");
    assert!(failures[0].contains("does not exist"), "{failures:?}");
}

#[test]
fn governance_codeowners_skips_globs_negations_and_suffixes() {
    let root = temp_root();
    // None of these name a decidable path; a vacuous failure would be worse
    // than a skip.
    write(
        root.path(),
        ".github/CODEOWNERS",
        "*  @owner\n!/docs  @owner\nsrc/  @owner\n/**/x.rs  @owner\n",
    );

    let mut failures = Vec::new();
    check_codeowners(root.path(), &mut failures);

    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn governance_absent_codeowners_is_left_to_the_file_check() {
    let root = temp_root();

    let mut failures = Vec::new();
    check_codeowners(root.path(), &mut failures);

    assert!(failures.is_empty(), "unexpected: {failures:?}");
}
