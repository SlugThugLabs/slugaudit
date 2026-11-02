//! The repo's test-file naming convention, shared by the gates that must
//! tell test code from production code. Pulled into each gate with `#[path]`.

use std::path::Path;

/// Whether `path` names a test file by the repo's conventions: a sibling
/// suite (`*_tests.rs`), the `tests.rs` shorthand, or a shared test fixture
/// (`test_support.rs`, `proptest.rs`). Test modules are `#[path]`-included
/// siblings, so the name is a reliable classifier.
pub(crate) fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.ends_with("_tests.rs")
                || name == "tests.rs"
                || name == "test_support.rs"
                || name == "proptest.rs"
        })
}

#[cfg(test)]
mod test_file_tests {
    use super::*;

    #[test]
    fn the_repos_test_file_names_are_recognized() {
        for name in [
            "query_tests.rs",
            "tests.rs",
            "test_support.rs",
            "proptest.rs",
        ] {
            assert!(is_test_file(&Path::new("src/x").join(name)), "{name}");
        }
    }

    #[test]
    fn production_file_names_are_not_test_files() {
        for name in ["query.rs", "tests_helper.rs", "attests.rs", "main.rs"] {
            assert!(!is_test_file(&Path::new("src/x").join(name)), "{name}");
        }
    }
}
