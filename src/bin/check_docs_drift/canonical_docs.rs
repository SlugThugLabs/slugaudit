//! Check 1: the canonical documentation set exists.
//!
//! The docs reset replaced the accumulated plan/report record with one
//! small, fixed set of documents (`README.md`, `AGENTS.md`,
//! `.planning/ARCHITECTURE.md`, `.planning/PLAN.md`,
//! `.planning/archive/DECISIONS.md`, `docs/CLIENTS.md`,
//! `docs/TROUBLESHOOTING.md`). This check holds the set fixed: every path
//! in [`CANONICAL_DOCS`] must exist as a file.
//!
//! Paths are written relative to the project root and are checked *at that
//! exact location* — deliberately not through the root → `.planning/` →
//! `.planning/archive/` fallback [`super::read`] uses. The fallback exists
//! so a prose reference to `ARCHITECTURE.md` keeps resolving wherever the
//! planning record lives; the canonical set, by contrast, is defined by
//! location, and a `PLAN.md` at the repository root is a different document
//! from `.planning/PLAN.md`.

use std::path::Path;

/// The canonical doc set, project-root relative. The order is presentation
/// order in failure messages only; membership is what is checked.
pub(super) const CANONICAL_DOCS: [&str; 8] = [
    "README.md",
    "AGENTS.md",
    ".planning/ARCHITECTURE.md",
    ".planning/MODULE_MAP.md",
    ".planning/PLAN.md",
    ".planning/archive/DECISIONS.md",
    "docs/CLIENTS.md",
    "docs/TROUBLESHOOTING.md",
];

/// Pushes one failure per missing canonical document.
///
/// A missing *required* document is a normal failure (exit code 1), not the
/// exit-code-2 "cannot read what I must verify" path of [`super::read`]:
/// this check's verdict *is* the absence of the file, so bailing out before
/// printing a verdict would be exactly the wrong reaction. `is_file` rather
/// than `exists`, because a directory sitting where a document is required
/// is not the document.
pub(super) fn check_canonical_docs(root: &Path, failures: &mut Vec<String>) {
    for doc in CANONICAL_DOCS {
        if !root.join(doc).is_file() {
            failures.push(format!(
                "required document `{doc}` is missing; the canonical doc set is fixed"
            ));
        }
    }
}
