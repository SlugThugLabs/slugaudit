//! Tests for the `*_tests.rs` count-claim check (`crate::test_file_count`).

use crate::test_file_count::check_test_file_count;
use crate::test_support::{temp_root, write};

#[test]
fn test_file_count_matches_the_claim() {
    let root = temp_root();
    write(root.path(), "src/foo_tests.rs", "#[test] fn t() {}\n");
    write(
        root.path(),
        "src/nested/bar_tests.rs",
        "#[test] fn u() {}\n",
    );
    write(root.path(), "src/plain.rs", "fn p() {}\n");
    write(
        root.path(),
        "ARCHITECTURE.md",
        "The 2 `*_tests.rs` files share this pattern; `cargo test --lib` runs\n",
    );
    let mut failures = Vec::new();
    check_test_file_count(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn test_file_count_mismatch_is_flagged() {
    let root = temp_root();
    write(root.path(), "src/foo_tests.rs", "#[test] fn t() {}\n");
    write(
        root.path(),
        "ARCHITECTURE.md",
        "The 5 `*_tests.rs` files share this pattern; `cargo test --lib` runs\n",
    );
    let mut failures = Vec::new();
    check_test_file_count(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("claims 5"), "{failures:?}");
}

#[test]
fn test_file_count_ignores_a_missing_src_dir() {
    let root = temp_root();
    write(
        root.path(),
        "ARCHITECTURE.md",
        "The 0 `*_tests.rs` files share this pattern; `cargo test --lib` runs\n",
    );
    let mut failures = Vec::new();
    check_test_file_count(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}
