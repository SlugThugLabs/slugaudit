use super::*;
use crate::graph::reference::ImportReference;
use crate::graph::resolver::ResolutionKind;
use crate::graph::rust_mod_parents::{
    RustModuleParents, insert_rust_mod_parent, insert_rust_mod_parent_segments,
    resolve_rust_path_attr, resolve_rust_path_attr_segments,
};
use crate::graph::test_support::paths;
use slugaudit_test_check::{TestResult, ensure, ensure_eq};

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

#[test]
fn path_attr_resolves_relative_to_declaring_files_parent_dir() -> TestResult {
    ensure_eq(
        &resolve_rust_path_attr("src/graph/resolve_rust.rs", "resolve_rust_tests.rs", ""),
        &Some("src/graph/resolve_rust_tests.rs".to_owned()),
        "sibling path attr resolves relative to src/graph",
    )?;
    ensure_eq(
        &resolve_rust_path_attr("src/graph/resolve_rust.rs", "../common/x.rs", ""),
        &Some("src/common/x.rs".to_owned()),
        "../ path attr walks up from declaring file's parent dir",
    )?;
    ensure_eq(
        &resolve_rust_path_attr("src/a.rs", "tests.rs", "inner"),
        &Some("src/a/inner/tests.rs".to_owned()),
        "inline_dir in non-mod.rs file joins under rust_module_dir",
    )?;
    ensure_eq(
        &resolve_rust_path_attr("src/a/mod.rs", "tests.rs", "inner"),
        &Some("src/a/inner/tests.rs".to_owned()),
        "inline_dir in mod.rs file joins under parent dir",
    )?;
    ensure_eq(
        &resolve_rust_path_attr("src/a.rs", "../../outside.rs", ""),
        &None,
        "path attr escaping repo root is ignored",
    )?;
    ensure_eq(
        &resolve_rust_path_attr("src/a.rs", "/abs/x.rs", ""),
        &None,
        "absolute path attr is ignored",
    )?;
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

    let glob = RustResolver.resolve_with_rust_parents(
        &refer("super::*"),
        "src/graph/resolve_rust_tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &glob.kind,
        &ResolutionKind::Resolved,
        "super::* in #[path] file resolves",
    )?;
    ensure_eq(
        &glob.to_relative_path.as_deref(),
        &Some("src/graph/resolve_rust.rs"),
        "super::* targets declaring file",
    )?;

    let item = RustResolver.resolve_with_rust_parents(
        &refer("super::HelperFn"),
        "src/graph/resolve_rust_tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &item.to_relative_path.as_deref(),
        &Some("src/graph/resolve_rust.rs"),
        "super::Item targets declaring file when no child module exists",
    )?;
    Ok(())
}

#[test]
fn super_child_mod_in_path_attr_file_prefers_submodule_of_declaring_file() -> TestResult {
    let known = paths(&["src/foo.rs", "src/foo_tests.rs", "src/foo/child_mod.rs"]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/foo.rs", "foo_tests.rs", "");

    let res = RustResolver.resolve_with_rust_parents(
        &refer("super::child_mod::Item"),
        "src/foo_tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &res.to_relative_path.as_deref(),
        &Some("src/foo/child_mod.rs"),
        "super::child_mod::Item resolves to submodule of declaring file",
    )?;
    Ok(())
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

    let sibling = RustResolver.resolve_with_rust_parents(
        &refer("super::super::other"),
        "src/a/b_tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &sibling.to_relative_path.as_deref(),
        &Some("src/a/other.rs"),
        "super::super::other resolves in parent module of declaring file",
    )?;

    let glob = RustResolver.resolve_with_rust_parents(
        &refer("super::super::*"),
        "src/a/b_tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &glob.to_relative_path.as_deref(),
        &Some("src/a/mod.rs"),
        "super::super::* resolves to grandparent module file",
    )?;
    Ok(())
}

#[test]
fn parent_relative_path_attr_resolves_super_to_declaring_file() -> TestResult {
    let known = paths(&["src/sub/host.rs", "src/common/x.rs"]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent(&mut parents, "src/sub/host.rs", "../common/x.rs", "");

    let res = RustResolver.resolve_with_rust_parents(
        &refer("super::*"),
        "src/common/x.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &res.to_relative_path.as_deref(),
        &Some("src/sub/host.rs"),
        "../common/x.rs super::* resolves back to src/sub/host.rs",
    )?;
    Ok(())
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

    // 1 super (< levels_in_declaring = 2): inline mod `inner` inside host.rs
    let one_super = RustResolver.resolve_with_rust_parents(
        &refer("super::*"),
        "src/a/host/inner/tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &one_super.to_relative_path.as_deref(),
        &Some("src/a/host.rs"),
        "single super inside an inline module resolves to the declaring file",
    )?;

    let inline_sibling = RustResolver.resolve_with_rust_parents(
        &refer("super::sibling::Item"),
        "src/a/host/inner/tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &inline_sibling.to_relative_path.as_deref(),
        &Some("src/a/host/inner/sibling.rs"),
        "single super inside an inline module resolves a sibling module under that inline module directory",
    )?;

    // 2 supers (== levels_in_declaring = 2): declaring file host.rs itself
    let two_super = RustResolver.resolve_with_rust_parents(
        &refer("super::super::*"),
        "src/a/host/inner/tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &two_super.to_relative_path.as_deref(),
        &Some("src/a/host.rs"),
        "double super from a depth-1 inline module targets the declaring file itself",
    )?;

    // 3 supers (> levels_in_declaring = 2): walks past host.rs to src/a/mod.rs
    let three_super = RustResolver.resolve_with_rust_parents(
        &refer("super::super::super::*"),
        "src/a/host/inner/tests.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &three_super.to_relative_path.as_deref(),
        &Some("src/a/mod.rs"),
        "triple super from a depth-1 inline module walks past the declaring file to its grandparent module",
    )?;
    Ok(())
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

    let in_inner = RustResolver.resolve_with_rust_parents(
        &refer("super::in_inner::Item"),
        "src/host/outer/inner/leaf.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &in_inner.to_relative_path.as_deref(),
        &Some("src/host/outer/inner/in_inner.rs"),
        "single super inside two nested inline modules resolves against outer/inner",
    )?;

    let in_outer = RustResolver.resolve_with_rust_parents(
        &refer("super::super::in_outer::Item"),
        "src/host/outer/inner/leaf.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &in_outer.to_relative_path.as_deref(),
        &Some("src/host/outer/in_outer.rs"),
        "double super inside two nested inline modules resolves against outer",
    )?;

    let host_glob = RustResolver.resolve_with_rust_parents(
        &refer("super::super::super::*"),
        "src/host/outer/inner/leaf.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &host_glob.to_relative_path.as_deref(),
        &Some("src/host.rs"),
        "triple super inside two nested inline modules resolves to the declaring host file",
    )?;
    Ok(())
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
    )?;
    Ok(())
}

#[test]
fn multi_segment_and_parent_relative_inline_segments_preserve_depth_and_normalize() -> TestResult {
    let known = paths(&[
        "src/sub/host.rs",
        "src/sub/host/a/b/leaf.rs",
        "src/sub/host/a/b/sibling.rs",
        "src/sub/common/leaf2.rs",
        "src/sub/common/helper.rs",
    ]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent_segments(
        &mut parents,
        "src/sub/host.rs",
        "leaf.rs",
        vec!["a/b".to_owned()],
    );
    insert_rust_mod_parent_segments(
        &mut parents,
        "src/sub/host.rs",
        "leaf2.rs",
        vec!["../common".to_owned()],
    );

    let sibling = RustResolver.resolve_with_rust_parents(
        &refer("super::sibling::Item"),
        "src/sub/host/a/b/leaf.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &sibling.to_relative_path.as_deref(),
        &Some("src/sub/host/a/b/sibling.rs"),
        "single super inside #[path = \"a/b\"] resolves sibling under src/sub/host/a/b",
    )?;

    let host_from_two = RustResolver.resolve_with_rust_parents(
        &refer("super::super::*"),
        "src/sub/host/a/b/leaf.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &host_from_two.to_relative_path.as_deref(),
        &Some("src/sub/host.rs"),
        "double super inside single multi-segment inline #[path = \"a/b\"] reaches declaring file",
    )?;

    let parent_rel = RustResolver.resolve_with_rust_parents(
        &refer("super::helper::Item"),
        "src/sub/common/leaf2.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &parent_rel.to_relative_path.as_deref(),
        &Some("src/sub/common/helper.rs"),
        "parent-relative inline #[path = \"../common\"] normalizes base_dir for super:: resolution",
    )?;
    Ok(())
}

#[test]
fn absolute_and_escaping_inline_segments_are_rejected() -> TestResult {
    ensure_eq(
        &resolve_rust_path_attr("src/a.rs", "tests.rs", "/abs/dir"),
        &None,
        "absolute inline_dir is rejected",
    )?;
    ensure_eq(
        &resolve_rust_path_attr_segments(
            "src/a.rs",
            "tests.rs",
            &["inner".to_owned(), "/abs".to_owned()],
        ),
        &None,
        "absolute inline segment is rejected",
    )?;
    ensure_eq(
        &resolve_rust_path_attr_segments("src/a.rs", "tests.rs", &["../../../escape".to_owned()]),
        &None,
        "escaping inline segment from non-mod.rs is rejected",
    )?;
    ensure_eq(
        &resolve_rust_path_attr_segments("src/lib.rs", "tests.rs", &["../../escape".to_owned()]),
        &None,
        "escaping inline segment from lib.rs is rejected",
    )?;
    Ok(())
}

#[test]
fn implicit_external_mod_under_inline_path_registers_both_file_and_mod_rs() -> TestResult {
    let known = paths(&[
        "src/lib.rs",
        "src/custom_dir/child.rs",
        "src/custom_dir/nested/mod.rs",
    ]);
    let mut parents = RustModuleParents::new();
    insert_rust_mod_parent_segments(
        &mut parents,
        "src/lib.rs",
        "child",
        vec!["custom_dir".to_owned()],
    );
    insert_rust_mod_parent_segments(
        &mut parents,
        "src/lib.rs",
        "nested",
        vec!["custom_dir".to_owned()],
    );

    let file_res = RustResolver.resolve_with_rust_parents(
        &refer("super::super::*"),
        "src/custom_dir/child.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &file_res.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "implicit child.rs under inline #[path] resolves back to declaring file",
    )?;

    let mod_res = RustResolver.resolve_with_rust_parents(
        &refer("super::super::*"),
        "src/custom_dir/nested/mod.rs",
        &known,
        &parents,
    );
    ensure_eq(
        &mod_res.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "implicit nested/mod.rs under inline #[path] resolves back to declaring file",
    )?;
    Ok(())
}
