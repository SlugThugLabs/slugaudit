// slugaudit-line-exception: approved-by=split-judge; reason=cohesive Rust import-resolution test suite covering crate, self, super, raw-ident, and confidence rules
use super::*;
use crate::graph::ImportReference;
use crate::graph::resolver::ResolutionKind;
use crate::graph::test_support::paths;
use slugaudit_test_check::{TestResult, ensure, ensure_eq};

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

#[test]
fn rust_crate_path_resolves_under_src() -> TestResult {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz::qux"), "src/main.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/baz/qux.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/baz/qux.rs\")",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("High"),
        "result.confidence == Some(\"High\")",
    )?;
    Ok(())
}

#[test]
fn rust_crate_path_resolves_mod_rs_form() -> TestResult {
    let known = paths(&["src/main.rs", "src/baz/mod.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz"), "src/main.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/baz/mod.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/baz/mod.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_crate_path_with_a_trailing_item_name_resolves_to_its_module_file() -> TestResult {
    let known = paths(&["src/main.rs", "src/helper.rs"]);
    let result = RustResolver.resolve(&refer("crate::helper::greet"), "src/main.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/helper.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/helper.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_crate_path_prefers_the_longest_directory_chain_that_actually_exists() -> TestResult {
    let known = paths(&["src/a.rs", "src/a/b.rs"]);
    let result = RustResolver.resolve(&refer("crate::a::b"), "src/main.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/a/b.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/a/b.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_super_is_a_low_confidence_heuristic() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/thing.rs"]);
    let result = RustResolver.resolve(&refer("super::thing"), "src/a/b.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("Low"),
        "result.confidence == Some(\"Low\")",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/thing.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/thing.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_self_is_a_low_confidence_heuristic() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/a/sibling.rs"]);
    let result = RustResolver.resolve(&refer("self::sibling"), "src/a/b.rs", &known);
    ensure_eq(
        &result.confidence,
        &Some("Low"),
        "result.confidence == Some(\"Low\")",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/a/b.rs"),
        "self::sibling in non-mod.rs file must not resolve to sibling file",
    )?;
    Ok(())
}

#[test]
fn rust_std_and_external_crates_are_external() -> TestResult {
    let known = paths(&["src/main.rs"]);
    ensure_eq(
        &RustResolver
            .resolve(&refer("std::collections::HashMap"), "src/main.rs", &known)
            .kind,
        &ResolutionKind::External,
        "RustResolver .resolve(&refer(\"std::collections::HashMap\"), \"src/main.rs\", &known) .kind == ResolutionKind::External",
    )?;
    ensure_eq(
        &RustResolver
            .resolve(&refer("serde::Serialize"), "src/main.rs", &known)
            .kind,
        &ResolutionKind::External,
        "RustResolver .resolve(&refer(\"serde::Serialize\"), \"src/main.rs\", &known) .kind == ResolutionKind::External",
    )?;
    Ok(())
}

#[test]
fn rust_crate_path_is_anchored_at_the_owning_crate_in_a_workspace() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/api/Cargo.toml",
        "crates/api/src/lib.rs",
        "crates/api/src/provider_error.rs",
        "crates/cli/Cargo.toml",
        "crates/cli/src/lib.rs",
    ]);
    let result = RustResolver.resolve(
        &refer("crate::provider_error::ProviderError"),
        "crates/api/src/registry.rs",
        &known,
    );
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("crates/api/src/provider_error.rs"),
        "crate:: must resolve within the importing file's own crate",
    )?;
    Ok(())
}

#[test]
fn rust_crate_path_still_falls_back_to_top_level_src() -> TestResult {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz::qux"), "src/main.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/baz/qux.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/baz/qux.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_glob_import_resolves_to_the_module_it_globs() -> TestResult {
    let known = paths(&["src/providers/mod.rs", "src/providers/openai.rs"]);
    let result = RustResolver.resolve(&refer("super::*"), "src/providers/openai.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/providers/mod.rs"),
        "a glob over the parent module must point at that module's file",
    )?;
    Ok(())
}

#[test]
fn rust_glob_import_resolves_against_a_named_module_too() -> TestResult {
    let known = paths(&["src/main.rs", "src/constants.rs"]);
    let result = RustResolver.resolve(&refer("super::constants::*"), "src/a/b.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/constants.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/constants.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_super_resolves_against_the_real_module_tree_not_the_grandparent_dir() -> TestResult {
    let known = paths(&[
        "src/providers/mod.rs",
        "src/providers/openai.rs",
        "src/providers/shared.rs",
    ]);
    let result = RustResolver.resolve(&refer("super::shared"), "src/providers/openai.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/providers/shared.rs"),
        "super:: from a non-mod.rs file must mean its own parent module",
    )?;
    Ok(())
}

#[test]
fn rust_repeated_super_walks_up_one_module_per_repetition() -> TestResult {
    let known = paths(&["src/a/mod.rs", "src/a/b/mod.rs", "src/a/b/c.rs"]);
    let result = RustResolver.resolve(&refer("super::super::*"), "src/a/b/c.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/a/mod.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/a/mod.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_super_from_a_mod_rs_file_still_resolves() -> TestResult {
    let known = paths(&["src/a/mod.rs", "src/thing.rs"]);
    let result = RustResolver.resolve(&refer("super::thing"), "src/a/mod.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/thing.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/thing.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_self_resolves_against_the_files_own_module_dir() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/a/b/child.rs"]);
    let result = RustResolver.resolve(&refer("self::child"), "src/a/b.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/a/b/child.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/a/b/child.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_bare_crate_glob_resolves_to_the_crate_root_file() -> TestResult {
    let known = paths(&["Cargo.toml", "src/lib.rs", "src/a.rs"]);
    let result = RustResolver.resolve(&refer("crate::*"), "src/a.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/lib.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_super_glob_from_a_top_level_module_resolves_to_the_crate_root() -> TestResult {
    let known = paths(&[
        "crates/api/Cargo.toml",
        "crates/api/src/lib.rs",
        "crates/api/src/codex_adapter.rs",
    ]);
    let result = RustResolver.resolve(
        &refer("super::*"),
        "crates/api/src/codex_adapter.rs",
        &known,
    );
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("crates/api/src/lib.rs"),
        "result.to_relative_path.as_deref() == Some(\"crates/api/src/lib.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_an_item_name_resolves_to_the_module_file_that_would_contain_it() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/a/mod.rs"]);
    let result = RustResolver.resolve(&refer("super::nonexistent"), "src/a/b.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/a/mod.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/a/mod.rs\")",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("Low"),
        "result.confidence == Some(\"Low\")",
    )?;
    Ok(())
}

#[test]
fn rust_an_import_with_no_candidate_file_at_all_is_unresolved() -> TestResult {
    let known = paths(&["src/lonely.rs"]);
    let result = RustResolver.resolve(&refer("super::whatever"), "src/lonely.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Unresolved,
        "result.kind == ResolutionKind::Unresolved",
    )?;
    Ok(())
}

#[test]
fn rust_crate_item_defined_in_the_crate_root_resolves_to_lib_rs() -> TestResult {
    let known = paths(&["src/lib.rs", "src/other.rs"]);
    let result = RustResolver.resolve(&refer("crate::Item"), "src/other.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/lib.rs\")",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("Low"),
        "result.confidence == Some(\"Low\")",
    )?;
    Ok(())
}

#[test]
fn rust_crate_item_defined_in_main_rs_resolves_to_main_rs() -> TestResult {
    let known = paths(&["src/main.rs", "src/other.rs"]);
    let result = RustResolver.resolve(&refer("crate::Item"), "src/other.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/main.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/main.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_2018_path_starting_with_a_local_module_resolves_to_its_file() -> TestResult {
    let known = paths(&["src/main.rs", "src/cli.rs"]);
    let result = RustResolver.resolve(&refer("cli::Args"), "src/main.rs", &known);
    ensure_eq(
        &result.kind,
        &ResolutionKind::Resolved,
        "result.kind == ResolutionKind::Resolved",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/cli.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/cli.rs\")",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("Low"),
        "result.confidence == Some(\"Low\")",
    )?;
    Ok(())
}

#[test]
fn rust_2018_local_module_of_a_non_root_file_resolves_under_its_module_dir() -> TestResult {
    let known = paths(&["src/a.rs", "src/a/inner.rs"]);
    let result = RustResolver.resolve(&refer("inner::Thing"), "src/a.rs", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/a/inner.rs"),
        "result.to_relative_path.as_deref() == Some(\"src/a/inner.rs\")",
    )?;
    Ok(())
}

#[test]
fn rust_pub_in_path_visibility_is_stripped_from_a_use_item() -> TestResult {
    let reference = RustResolver.extract_reference("pub(in crate::a) use crate::b::C;");
    ensure_eq(
        &reference.map(|r| r.text),
        &Some("crate::b::C".to_owned()),
        "reference.map(|r| r.text) == Some(\"crate::b::C\".to_owned())",
    )?;
    let plain = RustResolver.extract_reference("pub use crate::b;");
    ensure_eq(
        &plain.map(|r| r.text),
        &Some("crate::b".to_owned()),
        "plain.map(|r| r.text) == Some(\"crate::b\".to_owned())",
    )?;
    ensure(
        RustResolver.extract_reference("pubfn use x;").is_none(),
        "RustResolver.extract_reference(\"pubfn use x;\").is_none()",
    )?;
    Ok(())
}

#[test]
fn rust_super_prefers_parent_module_file_over_grandparent_or_root_build_rs() -> TestResult {
    let known = paths(&["src/lib.rs", "src/a.rs", "build.rs"]);
    let from_top = RustResolver.resolve(&refer("super::build"), "src/a.rs", &known);
    ensure_eq(
        &from_top.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "super::build from src/a.rs must resolve to src/lib.rs, not root build.rs",
    )?;

    let no_lib = paths(&[
        "src/a.rs",
        "build.rs",
        "crates/api/src/a.rs",
        "crates/api/build.rs",
    ]);
    ensure_eq(
        &RustResolver
            .resolve(&refer("super::build"), "src/a.rs", &no_lib)
            .kind,
        &ResolutionKind::Unresolved,
        "super::build must not climb above src into repo root",
    )?;
    ensure_eq(
        &RustResolver
            .resolve(&refer("super::build"), "crates/api/src/a.rs", &no_lib)
            .kind,
        &ResolutionKind::Unresolved,
        "super::build must not climb above crates/api/src into crate root",
    )?;

    let nested = paths(&["src/a/mod.rs", "src/a/b.rs", "src/item.rs"]);
    let from_nested = RustResolver.resolve(&refer("super::item"), "src/a/b.rs", &nested);
    ensure_eq(
        &from_nested.to_relative_path.as_deref(),
        &Some("src/a/mod.rs"),
        "super::item from src/a/b.rs must resolve to src/a/mod.rs before grandparent sibling",
    )?;
    Ok(())
}

#[test]
fn rust_raw_identifier_segments_strip_r_hash_prefix() -> TestResult {
    let known = paths(&["src/lib.rs", "src/match.rs", "src/type/mod.rs"]);
    let file_mod = RustResolver.resolve(&refer("crate::r#match::Foo"), "src/lib.rs", &known);
    ensure_eq(
        &file_mod.to_relative_path.as_deref(),
        &Some("src/match.rs"),
        "crate::r#match::Foo resolves to src/match.rs",
    )?;
    let dir_mod = RustResolver.resolve(&refer("crate::r#type::Bar"), "src/lib.rs", &known);
    ensure_eq(
        &dir_mod.to_relative_path.as_deref(),
        &Some("src/type/mod.rs"),
        "crate::r#type::Bar resolves to src/type/mod.rs",
    )?;
    Ok(())
}

#[test]
fn rust_leading_double_colon_path_is_always_external() -> TestResult {
    let known = paths(&["src/lib.rs", "src/foo.rs"]);
    let res = RustResolver.resolve(&refer("::foo::Bar"), "src/lib.rs", &known);
    ensure_eq(
        &res.kind,
        &ResolutionKind::External,
        "leading :: path must resolve as External even when local foo.rs exists",
    )?;
    Ok(())
}

#[test]
fn rust_crate_shortened_prefix_stamps_low_confidence_while_exact_module_stamps_high() -> TestResult
{
    let known = paths(&["src/lib.rs", "src/a.rs", "src/a/b.rs"]);

    let bare_crate = RustResolver.resolve(&refer("crate"), "src/a.rs", &known);
    ensure_eq(
        &(
            bare_crate.to_relative_path.as_deref(),
            bare_crate.confidence,
        ),
        &(Some("src/lib.rs"), Some("High")),
        "bare crate resolves to src/lib.rs with High confidence",
    )?;

    let exact = RustResolver.resolve(&refer("crate::a::b"), "src/lib.rs", &known);
    ensure_eq(
        &(exact.to_relative_path.as_deref(), exact.confidence),
        &(Some("src/a/b.rs"), Some("High")),
        "exact module match crate::a::b carries High confidence",
    )?;

    let shortened_one = RustResolver.resolve(&refer("crate::a::b::c"), "src/lib.rs", &known);
    ensure_eq(
        &(
            shortened_one.to_relative_path.as_deref(),
            shortened_one.confidence,
        ),
        &(Some("src/a/b.rs"), Some("Low")),
        "shortened prefix crate::a::b::c carries Low confidence because adding src/a/b/c.rs would retarget it",
    )?;

    let shortened_two =
        RustResolver.resolve(&refer("crate::a::missing::Item"), "src/lib.rs", &known);
    ensure_eq(
        &(
            shortened_two.to_relative_path.as_deref(),
            shortened_two.confidence,
        ),
        &(Some("src/a.rs"), Some("Low")),
        "multi-segment shortening crate::a::missing::Item carries Low confidence",
    )?;
    Ok(())
}

#[test]
fn rust_self_in_dual_lib_main_and_sibling_files_never_crosses_modules() -> TestResult {
    let known = paths(&["src/lib.rs", "src/main.rs", "src/a.rs", "src/sibling.rs"]);

    let main_item = RustResolver.resolve(&refer("self::Item"), "src/main.rs", &known);
    ensure_eq(
        &(main_item.to_relative_path.as_deref(), main_item.confidence),
        &(Some("src/main.rs"), Some("Low")),
        "self::Item in src/main.rs resolves to src/main.rs, not src/lib.rs",
    )?;

    let main_bare = RustResolver.resolve(&refer("self"), "src/main.rs", &known);
    ensure_eq(
        &(main_bare.to_relative_path.as_deref(), main_bare.confidence),
        &(Some("src/main.rs"), Some("High")),
        "bare self in src/main.rs resolves to src/main.rs with High confidence",
    )?;

    let main_glob = RustResolver.resolve(&refer("self::*"), "src/main.rs", &known);
    ensure_eq(
        &(main_glob.to_relative_path.as_deref(), main_glob.confidence),
        &(Some("src/main.rs"), Some("High")),
        "self::* in src/main.rs resolves to src/main.rs with High confidence",
    )?;

    let a_self = RustResolver.resolve(&refer("self::sibling"), "src/a.rs", &known);
    ensure_eq(
        &(a_self.to_relative_path.as_deref(), a_self.confidence),
        &(Some("src/a.rs"), Some("Low")),
        "self::sibling in src/a.rs never resolves to sibling src/sibling.rs",
    )?;
    Ok(())
}

#[test]
fn extern_crate_statements_resolve_without_syntax_unmodeled() -> TestResult {
    use crate::graph::{WorkspaceCrates, build_workspace_crates, resolve_imports_full};
    use slugaudit_test_check::Check;

    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "crates/my_crate/Cargo.toml",
        "crates/my_crate/src/lib.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [
            ("Cargo.toml", "[package]\nname = \"root\"\n"),
            (
                "crates/my_crate/Cargo.toml",
                "[package]\nname = \"my-crate\"\n",
            ),
        ],
        &known,
    );
    let edges = resolve_imports_full(
        "rust",
        "src/lib.rs",
        &[
            "extern crate alloc;".to_owned(),
            "extern crate my_crate as mc;".to_owned(),
            "extern crate self as my_self;".to_owned(),
        ],
        &known,
        &RustModuleParents::new(),
        &ws_crates,
    );
    ensure_eq(
        &edges.len(),
        &2,
        "extern crate self emits 0 edges while alloc and my_crate emit 2 edges",
    )?;

    let alloc_edge = edges.first().check("missing alloc edge")?;
    ensure_eq(
        &(alloc_edge.resolution_kind, alloc_edge.syntax_unmodeled),
        &("External", false),
        "extern crate alloc; is External and modeled",
    )?;

    let ws_edge = edges.get(1).check("missing my_crate edge")?;
    ensure_eq(
        &(
            ws_edge.resolution_kind,
            ws_edge.to_relative_path.as_deref(),
            ws_edge.confidence,
            ws_edge.syntax_unmodeled,
        ),
        &(
            "Resolved",
            Some("crates/my_crate/src/lib.rs"),
            Some("High"),
            false,
        ),
        "extern crate my_crate as mc; resolves to workspace crate lib.rs",
    )?;

    let empty_ws_edges = resolve_imports_full(
        "rust",
        "src/lib.rs",
        &["extern crate self as my_self;".to_owned()],
        &known,
        &RustModuleParents::new(),
        &WorkspaceCrates::new(),
    );
    ensure(
        empty_ws_edges.is_empty(),
        "extern crate self emits no edges and is not flagged syntax_unmodeled",
    )?;
    Ok(())
}

#[test]
fn custom_test_targets_resolve_crate_helper_crate_root_item_and_super_root_item() -> TestResult {
    use crate::graph::build_workspace_crates;

    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/helper.rs",
        "custom_tests/it_entry.rs",
        "custom_tests/helper.rs",
        "tests/helper.rs",
        "tests/group/it.rs",
        "tests/group/helper.rs",
    ]);
    let ws = build_workspace_crates(
        [(
            "Cargo.toml",
            "[package]\nname = \"app\"\n\
             [[test]]\nname = \"custom\"\npath = \"custom_tests/it_entry.rs\"\n\
             [[test]]\nname = \"group\"\npath = \"tests/group/it.rs\"\n",
        )],
        &known,
    );
    let parents = RustModuleParents::new();

    for (from, spec, want) in [
        (
            "custom_tests/it_entry.rs",
            "crate::helper",
            "custom_tests/helper.rs",
        ),
        (
            "custom_tests/helper.rs",
            "crate::helper",
            "custom_tests/helper.rs",
        ),
        (
            "custom_tests/helper.rs",
            "crate::RootItem",
            "custom_tests/it_entry.rs",
        ),
        (
            "custom_tests/helper.rs",
            "super::RootItem",
            "custom_tests/it_entry.rs",
        ),
        (
            "tests/group/it.rs",
            "crate::helper",
            "tests/group/helper.rs",
        ),
        (
            "tests/group/helper.rs",
            "crate::helper",
            "tests/group/helper.rs",
        ),
        (
            "tests/group/helper.rs",
            "crate::RootItem",
            "tests/group/it.rs",
        ),
        (
            "tests/group/helper.rs",
            "super::RootItem",
            "tests/group/it.rs",
        ),
    ] {
        let res = RustResolver.resolve_with_rust_context(&refer(spec), from, &known, &parents, &ws);
        ensure_eq(
            &res.to_relative_path.as_deref(),
            &Some(want),
            format!("{from} resolves {spec} to {want}"),
        )?;
    }
    Ok(())
}

#[test]
fn rust_inline_mod_import_resolution_handles_super_self_and_local_paths() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/helper.rs",
        "src/sub.rs",
        "src/foo.rs",
        "src/foo/sub.rs",
        "src/foo/inner/sub.rs",
    ]);
    let parents = RustModuleParents::new();
    let ws = WorkspaceCrates::new();
    let tests_mod = vec!["tests".to_owned()];
    let inner_mod = vec!["inner".to_owned()];

    for (from, inline, spec, want_path, want_conf) in [
        ("src/lib.rs", &tests_mod, "super::*", "src/lib.rs", "High"),
        (
            "src/lib.rs",
            &tests_mod,
            "super::helper::X",
            "src/helper.rs",
            "Low",
        ),
        ("src/lib.rs", &tests_mod, "super::sub", "src/sub.rs", "High"),
        ("src/foo.rs", &tests_mod, "super::*", "src/foo.rs", "High"),
        (
            "src/foo.rs",
            &tests_mod,
            "super::sub::Item",
            "src/foo/sub.rs",
            "Low",
        ),
        (
            "src/foo.rs",
            &tests_mod,
            "super::super::Bar",
            "src/lib.rs",
            "Low",
        ),
        (
            "src/foo.rs",
            &inner_mod,
            "self::sub::Thing",
            "src/foo/inner/sub.rs",
            "Low",
        ),
        (
            "src/foo.rs",
            &inner_mod,
            "sub::Thing",
            "src/foo/inner/sub.rs",
            "Low",
        ),
    ] {
        let res = RustResolver::resolve_in_inline_scope(
            &refer(spec),
            from,
            inline,
            &known,
            &parents,
            &ws,
        );
        ensure_eq(
            &(res.kind, res.to_relative_path.as_deref(), res.confidence),
            &(ResolutionKind::Resolved, Some(want_path), Some(want_conf)),
            format!("{from} ({inline:?}): {spec} -> {want_path} ({want_conf})"),
        )?;
    }
    Ok(())
}
