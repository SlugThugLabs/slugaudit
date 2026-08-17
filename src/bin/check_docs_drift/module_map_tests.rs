//! Tests for the module-map existence check (`crate::module_map`).

use crate::module_map::check_module_map;
use crate::test_support::{temp_root, write, write_module_map};

#[test]
fn module_map_with_existing_files_passes() {
    let root = temp_root();
    write_module_map(root.path());
    let mut failures = Vec::new();
    check_module_map(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn module_map_flags_a_stale_rename() {
    let root = temp_root();
    // The real file was renamed from `reconcile.rs` to
    // `reconcile_new.rs`, but the module map still references the old
    // name — the rename the gate exists to catch. (Unlike
    // `write_module_map`, only the *new* name exists on disk.)
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");
    write(root.path(), "src/sync/reconcile_new.rs", "pub fn r() {}\n");
    write(
        root.path(),
        "ARCHITECTURE.md",
        "\n## Module map\n\n```\nsrc/\n├── lib.rs\n├── sync/\n│   └── reconcile.rs\n```\n",
    );
    let mut failures = Vec::new();
    check_module_map(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("reconcile.rs"), "{failures:?}");
}

#[test]
fn module_map_reports_a_missing_directory() {
    let root = temp_root();
    write(root.path(), "src/lib.rs", "pub fn a() {}\n");
    // A bare directory entry with no children, so the gate reports
    // exactly the one missing directory (a file under a missing
    // directory would be reported separately as well).
    write(
        root.path(),
        "ARCHITECTURE.md",
        "\n## Module map\n\n```\nsrc/\n├── lib.rs\n├── missing/\n```\n",
    );
    let mut failures = Vec::new();
    check_module_map(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("missing"), "{failures:?}");
}

#[test]
fn module_map_missing_block_is_flagged() {
    let root = temp_root();
    write(root.path(), "ARCHITECTURE.md", "# no map here\n");
    let mut failures = Vec::new();
    check_module_map(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("not found"), "{failures:?}");
}
