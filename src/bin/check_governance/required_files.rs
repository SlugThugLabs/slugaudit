//! Check 1: the fixed governance shell must exist.
//!
//! The governance shell landed as five files (`.github/CODEOWNERS`,
//! `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`)
//! and is deliberately closed: renaming or dropping one is a governance
//! decision, not a file move, so the gate holds the set fixed the same way
//! `check_docs_drift` holds its canonical doc set.

use std::path::Path;

/// The required governance files, project-root relative. Order is
/// presentation order in failure messages only; membership is what is
/// checked.
pub(super) const REQUIRED_FILES: [&str; 5] = [
    ".github/CODEOWNERS",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "CHANGELOG.md",
];

/// Pushes one failure per missing required file.
///
/// `is_file` rather than `exists`, because a directory sitting where a
/// governance document is required is not the document.
pub(super) fn check_required_files(root: &Path, failures: &mut Vec<String>) {
    for file in REQUIRED_FILES {
        if !root.join(file).is_file() {
            failures.push(format!(
                "required governance file `{file}` is missing; the governance shell is a fixed set"
            ));
        }
    }
}
