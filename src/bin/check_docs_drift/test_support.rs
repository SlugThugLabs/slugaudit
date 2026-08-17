//! Shared test fixtures for the docs-drift gate. Test-only; wired from
//! `main.rs` behind `#[cfg(test)]` so no test module duplicates them.

use std::fs;
use std::path::Path;

pub(super) fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write fixture");
}

pub(super) fn temp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

/// A minimal module map whose referenced files actually exist, so a
/// stale entry is the *only* difference a test needs to introduce.
pub(super) fn write_module_map(root: &Path) {
    write(root, "src/lib.rs", "pub fn a() {}\n");
    write(root, "src/sync/reconcile.rs", "pub fn r() {}\n");
    write(
        root,
        "ARCHITECTURE.md",
        "\n## Module map\n\n```\nsrc/\n├── lib.rs\n├── sync/\n│   └── reconcile.rs\n```\n",
    );
}
