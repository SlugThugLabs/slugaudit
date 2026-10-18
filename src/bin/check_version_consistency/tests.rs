//! Tests for the version-consistency gate: the parsers' rules and the gate's
//! verdict on a fixture whose four copies agree or disagree.

use std::path::Path;

use tempfile::{TempDir, tempdir};

use super::manifests;

const CARGO: &str = "version = \"1.0.10\"\ntree-sitter-language-pack = \"=1.14.3\"\n";
const PLUGIN: &str = "{\n  \"version\": \"1.0.10\"\n}\n";
const CHANGELOG: &str = "# Changelog\n\n## [Unreleased]\n\n## [1.0.10] - 2026-09-06\n";
const PARSE: &str = "pub const PACK_VERSION: &str = \"1.14.3\";\n";

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

fn agreeing_fixture(root: &Path) {
    write(root, "Cargo.toml", CARGO);
    write(root, "lhm.plugin.json", PLUGIN);
    write(root, "CHANGELOG.md", CHANGELOG);
    write(root, "src/parse/mod.rs", PARSE);
}

fn failures(root: &Path) -> Vec<String> {
    manifests::check(root).expect("all four fixture files readable")
}

#[test]
fn version_consistency_agreeing_copies_pass() {
    let root = temp_root();
    agreeing_fixture(root.path());

    assert!(failures(root.path()).is_empty());
}

#[test]
fn version_consistency_flags_cargo_and_plugin_disagreement() {
    let root = temp_root();
    agreeing_fixture(root.path());
    write(
        root.path(),
        "lhm.plugin.json",
        "{\n  \"version\": \"1.0.11\"\n}\n",
    );

    let failures = failures(root.path());

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("Cargo.toml"), "{failures:?}");
    assert!(failures[0].contains("lhm.plugin.json"), "{failures:?}");
}

#[test]
fn version_consistency_flags_a_stale_changelog_heading() {
    let root = temp_root();
    agreeing_fixture(root.path());
    write(
        root.path(),
        "CHANGELOG.md",
        "# Changelog\n\n## [1.0.9] - 2026-08-01\n",
    );

    let failures = failures(root.path());

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("1.0.9"), "{failures:?}");
}

#[test]
fn version_consistency_flags_a_pack_version_disagreement() {
    let root = temp_root();
    agreeing_fixture(root.path());
    write(
        root.path(),
        "src/parse/mod.rs",
        "pub const PACK_VERSION: &str = \"1.14.4\";\n",
    );

    let failures = failures(root.path());

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("PACK_VERSION"), "{failures:?}");
}

#[test]
fn version_consistency_rejects_a_non_exact_pack_pin() {
    let root = temp_root();
    agreeing_fixture(root.path());
    write(
        root.path(),
        "Cargo.toml",
        "version = \"1.0.10\"\ntree-sitter-language-pack = \"1.14.3\"\n",
    );

    let failures = failures(root.path());

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("not exact"), "{failures:?}");
}

#[test]
fn version_consistency_reports_malformed_plugin_json() {
    let root = temp_root();
    agreeing_fixture(root.path());
    write(root.path(), "lhm.plugin.json", "{ not json\n");

    let failures = failures(root.path());

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("not valid JSON"), "{failures:?}");
}

#[test]
fn version_consistency_unreadable_file_is_an_error_not_a_verdict() {
    let root = temp_root();
    agreeing_fixture(root.path());
    std::fs::remove_file(root.path().join("CHANGELOG.md")).expect("remove fixture");

    let error = manifests::check(root.path()).expect_err("a missing CHANGELOG.md is an error");

    assert!(error.contains("CHANGELOG.md"), "{error}");
}

#[test]
fn version_consistency_changelog_ignores_the_unreleased_heading() {
    assert_eq!(
        manifests::changelog_version("# Changelog\n\n## [Unreleased]\n\n## [2.3.4] - 2026-01-01\n"),
        Some("2.3.4".to_owned())
    );
}
