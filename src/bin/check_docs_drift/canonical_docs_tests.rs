//! Tests for the canonical-doc-set existence check
//! (`crate::canonical_docs`).

use crate::canonical_docs::{CANONICAL_DOCS, check_canonical_docs};
use crate::test_support::{temp_root, write};

/// Materialises the whole canonical set, so a test can remove exactly the
/// one document it cares about.
fn write_canonical_docs(root: &std::path::Path) {
    for doc in CANONICAL_DOCS {
        write(root, doc, "# doc\n");
    }
}

#[test]
fn canonical_docs_complete_set_passes() {
    let root = temp_root();
    write_canonical_docs(root.path());
    let mut failures = Vec::new();
    check_canonical_docs(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn canonical_docs_reports_each_missing_document_by_path() {
    let root = temp_root();
    write_canonical_docs(root.path());
    std::fs::remove_file(root.path().join("docs/CLIENTS.md")).expect("remove fixture");
    std::fs::remove_file(root.path().join(".planning/PLAN.md")).expect("remove fixture");
    let mut failures = Vec::new();
    check_canonical_docs(root.path(), &mut failures);
    assert_eq!(failures.len(), 2, "unexpected: {failures:?}");
    assert!(
        failures.iter().any(|f| f.contains("docs/CLIENTS.md")),
        "{failures:?}"
    );
    assert!(
        failures.iter().any(|f| f.contains(".planning/PLAN.md")),
        "{failures:?}"
    );
}

#[test]
fn canonical_docs_rejects_a_fallback_location() {
    let root = temp_root();
    write_canonical_docs(root.path());
    // `read` would resolve a bare `README.md`, but the canonical set is
    // defined by exact location: a root `README.md` is not
    // `.planning/ARCHITECTURE.md`'s stand-in, and a document parked under
    // `.planning/` does not satisfy the root entry.
    std::fs::remove_file(root.path().join("README.md")).expect("remove fixture");
    write(root.path(), ".planning/README.md", "# doc\n");
    let mut failures = Vec::new();
    check_canonical_docs(root.path(), &mut failures);
    assert_eq!(failures.len(), 1, "unexpected: {failures:?}");
    assert!(failures[0].contains("`README.md`"), "{failures:?}");
}

#[test]
fn canonical_docs_rejects_a_directory_in_place_of_a_document() {
    let root = temp_root();
    write_canonical_docs(root.path());
    std::fs::remove_file(root.path().join("docs/TROUBLESHOOTING.md")).expect("remove fixture");
    std::fs::create_dir(root.path().join("docs/TROUBLESHOOTING.md")).expect("create fixture dir");
    let mut failures = Vec::new();
    check_canonical_docs(root.path(), &mut failures);
    assert_eq!(failures.len(), 1, "unexpected: {failures:?}");
    assert!(
        failures[0].contains("docs/TROUBLESHOOTING.md"),
        "{failures:?}"
    );
}
