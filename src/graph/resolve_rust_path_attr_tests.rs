use super::*;
use crate::graph::resolver::ResolutionKind;
use crate::graph::rust_mod_parents::{
    RustModuleParents, insert_rust_mod_parent_resolved, insert_rust_mod_parent_segments_in_context,
};
use crate::graph::rust_path_attr::resolve_rust_path_attr_segments;
use crate::graph::test_support::paths;
use crate::graph::{ImportReference, WorkspaceCrates};
use slugaudit_test_check::{TestResult, ensure, ensure_eq};
use std::collections::HashSet;

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

fn split_inline_dir(inline_dir: &str) -> Vec<String> {
    inline_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn insert_rust_mod_parent_segments(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_segments: Vec<String>,
) {
    let child_path = resolve_rust_path_attr_segments(declaring_file, path_attr, &inline_segments);
    insert_rust_mod_parent_resolved(
        parents,
        declaring_file,
        path_attr,
        inline_segments,
        child_path,
    );
}

fn insert_rust_mod_parent(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_dir: &str,
) {
    if inline_dir.starts_with('/') {
        return;
    }
    insert_rust_mod_parent_segments(
        parents,
        declaring_file,
        path_attr,
        split_inline_dir(inline_dir),
    );
}

fn add_parent_segments(
    parents: &mut RustModuleParents,
    declaring_file: &str,
    path_attr: &str,
    inline_segments: &[&str],
) {
    let owned = inline_segments.iter().copied().map(str::to_owned).collect();
    insert_rust_mod_parent_segments(parents, declaring_file, path_attr, owned);
}

fn assert_parent_target(
    known: &HashSet<&str>,
    parents: &RustModuleParents,
    cases: &[(&str, &str, &str)],
) -> TestResult {
    for &(importing_file, spec, expected_target) in cases {
        let res =
            RustResolver.resolve_with_rust_parents(&refer(spec), importing_file, known, parents);
        ensure_eq(
            &(res.kind, res.to_relative_path.as_deref()),
            &(ResolutionKind::Resolved, Some(expected_target)),
            format!("{importing_file}: {spec} -> {expected_target}"),
        )?;
    }
    Ok(())
}

fn assert_context_target(
    known: &HashSet<&str>,
    parents: &RustModuleParents,
    ws: &WorkspaceCrates,
    cases: &[(&str, &str, &str, &str)],
) -> TestResult {
    for &(importing_file, spec, expected_target, expected_conf) in cases {
        let res = RustResolver.resolve_with_rust_context(
            &refer(spec),
            importing_file,
            known,
            parents,
            ws,
        );
        ensure_eq(
            &(res.to_relative_path.as_deref(), res.confidence),
            &(Some(expected_target), Some(expected_conf)),
            format!("{importing_file}: {spec} -> {expected_target} ({expected_conf})"),
        )?;
    }
    Ok(())
}

#[test]
fn super_glob_and_item_in_path_attr_file_resolve_to_declaring_file() -> TestResult {
    let known = paths(&[
        "src/graph/resolve_rust.rs",
        "src/graph/resolve_rust_tests.rs",
    ]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(
        &mut parents,
        "src/graph/resolve_rust.rs",
        "resolve_rust_tests.rs",
        "",
    );
    assert_parent_target(
        &known,
        &parents,
        &[
            (
                "src/graph/resolve_rust_tests.rs",
                "super::*",
                "src/graph/resolve_rust.rs",
            ),
            (
                "src/graph/resolve_rust_tests.rs",
                "super::HelperFn",
                "src/graph/resolve_rust.rs",
            ),
        ],
    )
}

#[test]
fn super_child_mod_in_path_attr_file_prefers_submodule_of_declaring_file() -> TestResult {
    let known = paths(&["src/foo.rs", "src/foo_tests.rs", "src/foo/child_mod.rs"]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/foo.rs", "foo_tests.rs", "");
    assert_parent_target(
        &known,
        &parents,
        &[(
            "src/foo_tests.rs",
            "super::child_mod::Item",
            "src/foo/child_mod.rs",
        )],
    )
}

#[test]
fn repeated_super_in_path_attr_file_walks_past_declaring_file() -> TestResult {
    let known = paths(&[
        "src/a/mod.rs",
        "src/a/b.rs",
        "src/a/b_tests.rs",
        "src/a/other.rs",
    ]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/a/b.rs", "b_tests.rs", "");
    assert_parent_target(
        &known,
        &parents,
        &[
            ("src/a/b_tests.rs", "super::super::other", "src/a/other.rs"),
            ("src/a/b_tests.rs", "super::super::*", "src/a/mod.rs"),
        ],
    )
}

#[test]
fn parent_relative_path_attr_resolves_super_to_declaring_file() -> TestResult {
    let known = paths(&["src/sub/host.rs", "src/common/x.rs"]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/sub/host.rs", "../common/x.rs", "");
    assert_parent_target(
        &known,
        &parents,
        &[("src/common/x.rs", "super::*", "src/sub/host.rs")],
    )
}

#[test]
fn inline_mod_path_attr_handles_all_super_levels() -> TestResult {
    let known = paths(&[
        "src/a/mod.rs",
        "src/a/host.rs",
        "src/a/host/inner/tests.rs",
        "src/a/host/inner/sibling.rs",
    ]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/a/host.rs", "tests.rs", "inner");
    assert_parent_target(
        &known,
        &parents,
        &[
            ("src/a/host/inner/tests.rs", "super::*", "src/a/host.rs"),
            (
                "src/a/host/inner/tests.rs",
                "super::sibling::Item",
                "src/a/host/inner/sibling.rs",
            ),
            (
                "src/a/host/inner/tests.rs",
                "super::super::*",
                "src/a/host.rs",
            ),
            (
                "src/a/host/inner/tests.rs",
                "super::super::super::*",
                "src/a/mod.rs",
            ),
        ],
    )
}

#[test]
fn nested_two_level_inline_mod_path_attr_resolves_each_enclosing_inline_dir() -> TestResult {
    let known = paths(&[
        "src/host.rs",
        "src/host/outer/inner/leaf.rs",
        "src/host/outer/inner/in_inner.rs",
        "src/host/outer/in_outer.rs",
    ]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/host.rs", "leaf.rs", "outer/inner");
    assert_parent_target(
        &known,
        &parents,
        &[
            (
                "src/host/outer/inner/leaf.rs",
                "super::in_inner::Item",
                "src/host/outer/inner/in_inner.rs",
            ),
            (
                "src/host/outer/inner/leaf.rs",
                "super::super::in_outer::Item",
                "src/host/outer/in_outer.rs",
            ),
            (
                "src/host/outer/inner/leaf.rs",
                "super::super::super::*",
                "src/host.rs",
            ),
        ],
    )
}

#[test]
fn cyclic_path_attrs_terminate_within_hop_cap() -> TestResult {
    let known = paths(&["src/a.rs", "src/b.rs"]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/a.rs", "b.rs", "");
    insert_rust_mod_parent(&mut parents, "src/b.rs", "a.rs", "");

    let deep_super = "super::".repeat(40);
    let spec = format!("{deep_super}Item");
    let res = RustResolver.resolve_with_rust_parents(&refer(&spec), "src/a.rs", &known, &parents);
    ensure(
        matches!(
            res.kind,
            ResolutionKind::Resolved | ResolutionKind::Unresolved
        ),
        "cyclic #[path] must terminate cleanly without hanging or panicking",
    )
}

#[test]
fn multi_segment_inline_segment_preserves_depth() -> TestResult {
    let known = paths(&[
        "src/sub/host.rs",
        "src/sub/host/a/b/leaf.rs",
        "src/sub/host/a/b/sibling.rs",
    ]);
    let mut parents = RustModuleParents::new();
    add_parent_segments(&mut parents, "src/sub/host.rs", "leaf.rs", &["a/b"]);
    assert_parent_target(
        &known,
        &parents,
        &[
            (
                "src/sub/host/a/b/leaf.rs",
                "super::sibling::Item",
                "src/sub/host/a/b/sibling.rs",
            ),
            (
                "src/sub/host/a/b/leaf.rs",
                "super::super::*",
                "src/sub/host.rs",
            ),
        ],
    )
}

#[test]
fn parent_relative_inline_segment_normalizes_base_dir() -> TestResult {
    let known = paths(&[
        "src/sub/host.rs",
        "src/sub/common/leaf2.rs",
        "src/sub/common/helper.rs",
    ]);
    let mut parents = RustModuleParents::new();
    add_parent_segments(&mut parents, "src/sub/host.rs", "leaf2.rs", &["../common"]);
    assert_parent_target(
        &known,
        &parents,
        &[(
            "src/sub/common/leaf2.rs",
            "super::helper::Item",
            "src/sub/common/helper.rs",
        )],
    )
}

#[test]
fn implicit_external_mod_under_inline_path_registers_both_file_and_mod_rs() -> TestResult {
    let known = paths(&[
        "src/lib.rs",
        "src/custom_dir/child.rs",
        "src/custom_dir/nested/mod.rs",
    ]);
    let mut parents = RustModuleParents::new();
    add_parent_segments(&mut parents, "src/lib.rs", "child", &["custom_dir"]);
    add_parent_segments(&mut parents, "src/lib.rs", "nested", &["custom_dir"]);
    assert_parent_target(
        &known,
        &parents,
        &[
            ("src/custom_dir/child.rs", "super::super::*", "src/lib.rs"),
            (
                "src/custom_dir/nested/mod.rs",
                "super::super::*",
                "src/lib.rs",
            ),
        ],
    )
}

#[test]
fn inline_path_attr_in_non_stem_crate_root_uses_effective_module_dir() -> TestResult {
    let known = paths(&[
        "tests/integration.rs",
        "tests/helpers/extra.rs",
        "tests/helpers/sibling.rs",
    ]);
    let ws = WorkspaceCrates::new();
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent_segments_in_context(
        &mut parents,
        "tests/integration.rs",
        "extra.rs",
        vec!["helpers".to_owned()],
        &known,
        &ws,
    );
    assert_context_target(
        &known,
        &parents,
        &ws,
        &[
            (
                "tests/helpers/extra.rs",
                "super::sibling::Item",
                "tests/helpers/sibling.rs",
                "Low",
            ),
            (
                "tests/helpers/extra.rs",
                "super::super::RootItem",
                "tests/integration.rs",
                "Low",
            ),
        ],
    )
}

#[test]
fn ordinary_submodule_under_path_attr_walks_super_super_to_declaring_file() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/foo.rs",
        "src/custom/bar.rs",
        "src/custom/bar/baz.rs",
    ]);
    let ws = WorkspaceCrates::new();
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent_segments_in_context(
        &mut parents,
        "src/foo.rs",
        "custom/bar.rs",
        Vec::new(),
        &known,
        &ws,
    );
    assert_context_target(
        &known,
        &parents,
        &ws,
        &[
            (
                "src/custom/bar/baz.rs",
                "super::super::*",
                "src/foo.rs",
                "High",
            ),
            (
                "src/custom/bar/baz.rs",
                "super::super::FooItem",
                "src/foo.rs",
                "Low",
            ),
        ],
    )
}

#[test]
fn ordinary_submodule_under_path_attr_in_integration_test_anchors_crate_to_test_root() -> TestResult
{
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "tests/it.rs",
        "shared/sub.rs",
        "shared/sub/leaf.rs",
    ]);
    let ws = WorkspaceCrates::new();
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent_segments_in_context(
        &mut parents,
        "tests/it.rs",
        "../shared/sub.rs",
        Vec::new(),
        &known,
        &ws,
    );
    assert_context_target(
        &known,
        &parents,
        &ws,
        &[("shared/sub/leaf.rs", "crate::Helper", "tests/it.rs", "Low")],
    )
}
