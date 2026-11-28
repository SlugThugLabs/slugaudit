use super::*;
use crate::graph::test_support::paths;
use slugaudit_test_check::{TestResult, ensure_eq};

fn resolve_rust_path_attr(
    declaring_file: &str,
    path_attr: &str,
    inline_segments: &[&str],
) -> Option<String> {
    let owned: Vec<String> = inline_segments.iter().copied().map(str::to_owned).collect();
    resolve_rust_path_attr_segments(declaring_file, path_attr, &owned)
}

type PathAttrCase<'a> = (&'a str, &'a str, &'a [&'a str], Option<&'a str>, &'a str);

fn assert_path_attr_cases(cases: &[PathAttrCase<'_>]) -> TestResult {
    for &(declaring_file, path_attr, inline_segments, expected, msg) in cases {
        ensure_eq(
            &resolve_rust_path_attr(declaring_file, path_attr, inline_segments).as_deref(),
            &expected,
            msg,
        )?;
    }
    Ok(())
}

#[test]
fn path_attr_resolves_relative_to_declaring_files_parent_dir() -> TestResult {
    assert_path_attr_cases(&[
        (
            "src/graph/resolve_rust.rs",
            "resolve_rust_tests.rs",
            &[],
            Some("src/graph/resolve_rust_tests.rs"),
            "sibling path attr resolves relative to src/graph",
        ),
        (
            "src/graph/resolve_rust.rs",
            "../common/x.rs",
            &[],
            Some("src/common/x.rs"),
            "../ path attr walks up from declaring file's parent dir",
        ),
        (
            "src/a.rs",
            "tests.rs",
            &["inner"],
            Some("src/a/inner/tests.rs"),
            "inline_dir in non-mod.rs file joins under rust_module_dir",
        ),
        (
            "src/a/mod.rs",
            "tests.rs",
            &["inner"],
            Some("src/a/inner/tests.rs"),
            "inline_dir in mod.rs file joins under parent dir",
        ),
        (
            "src/a.rs",
            "../../outside.rs",
            &[],
            None,
            "path attr escaping repo root is ignored",
        ),
        (
            "src/a.rs",
            "/abs/x.rs",
            &[],
            None,
            "absolute path attr is ignored",
        ),
    ])
}

#[test]
fn absolute_and_escaping_inline_segments_are_rejected() -> TestResult {
    assert_path_attr_cases(&[
        (
            "src/a.rs",
            "tests.rs",
            &["/abs/dir"],
            None,
            "absolute inline_dir is rejected",
        ),
        (
            "src/a.rs",
            "tests.rs",
            &["inner", "/abs"],
            None,
            "absolute inline segment is rejected",
        ),
        (
            "src/a.rs",
            "tests.rs",
            &["../../../escape"],
            None,
            "escaping inline segment from non-mod.rs is rejected",
        ),
        (
            "src/lib.rs",
            "tests.rs",
            &["../../escape"],
            None,
            "escaping inline segment from lib.rs is rejected",
        ),
    ])
}

#[test]
fn path_attr_in_context_uses_effective_module_dir_for_non_stem_crate_root() -> TestResult {
    let known = paths(&[
        "tests/integration.rs",
        "tests/helpers/extra.rs",
        "tests/helpers/sibling.rs",
    ]);
    let ws = WorkspaceCrates::new();
    let inline = vec!["helpers".to_owned()];
    ensure_eq(
        &resolve_rust_path_attr_segments_in_context(
            "tests/integration.rs",
            "extra.rs",
            &inline,
            &known,
            &ws,
        ),
        &Some("tests/helpers/extra.rs".to_owned()),
        "inline #[path] in tests/integration.rs resolves under tests/helpers/",
    )
}
