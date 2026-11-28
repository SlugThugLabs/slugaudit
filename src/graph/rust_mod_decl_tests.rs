use super::*;
use crate::graph::test_support::paths;
use slugaudit_test_check::{TestResult, ensure_eq};

#[test]
fn inline_path_attr_mod_decl_in_non_stem_crate_root_uses_effective_module_dir() -> TestResult {
    let known = paths(&[
        "tests/integration.rs",
        "tests/helpers/extra.rs",
        "tests/helpers/sibling.rs",
    ]);
    let ws = WorkspaceCrates::new();
    let inline = vec!["helpers".to_owned()];
    let decl_edge = resolve_rust_mod_declaration(
        "tests/integration.rs",
        "extra",
        Some("extra.rs"),
        &inline,
        &known,
        &ws,
    );
    ensure_eq(
        &decl_edge.to_relative_path.as_deref(),
        &Some("tests/helpers/extra.rs"),
        "mod extra; under inline helpers in tests/integration.rs resolves to tests/helpers/extra.rs",
    )
}
