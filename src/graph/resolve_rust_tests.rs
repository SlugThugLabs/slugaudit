use super::*;
use crate::graph::reference::ImportReference;
use crate::graph::resolver::ResolutionKind;
use crate::graph::test_support::paths;
use slugaudit_test_check::TestResult;

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

#[test]
fn rust_crate_path_resolves_under_src() -> TestResult {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz::qux"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/qux.rs"));
    assert_eq!(result.confidence, Some("High"));
    Ok(())
}

#[test]
fn rust_crate_path_resolves_mod_rs_form() -> TestResult {
    let known = paths(&["src/main.rs", "src/baz/mod.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/mod.rs"));
    Ok(())
}

#[test]
fn rust_crate_path_with_a_trailing_item_name_resolves_to_its_module_file() -> TestResult {
    let known = paths(&["src/main.rs", "src/helper.rs"]);
    let result = RustResolver.resolve(&refer("crate::helper::greet"), "src/main.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/helper.rs"));
    Ok(())
}

#[test]
fn rust_crate_path_prefers_the_longest_directory_chain_that_actually_exists() -> TestResult {
    let known = paths(&["src/a.rs", "src/a/b.rs"]);
    let result = RustResolver.resolve(&refer("crate::a::b"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/b.rs"));
    Ok(())
}

#[test]
fn rust_super_is_a_low_confidence_heuristic() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/thing.rs"]);
    let result = RustResolver.resolve(&refer("super::thing"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.confidence, Some("Low"));
    assert_eq!(result.to_relative_path.as_deref(), Some("src/thing.rs"));
    Ok(())
}

#[test]
fn rust_self_is_a_low_confidence_heuristic() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/a/sibling.rs"]);
    let result = RustResolver.resolve(&refer("self::sibling"), "src/a/b.rs", &known);
    assert_eq!(result.confidence, Some("Low"));
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/sibling.rs"));
    Ok(())
}

#[test]
fn rust_std_and_external_crates_are_external() -> TestResult {
    let known = paths(&["src/main.rs"]);
    assert_eq!(
        RustResolver
            .resolve(&refer("std::collections::HashMap"), "src/main.rs", &known)
            .kind,
        ResolutionKind::External
    );
    assert_eq!(
        RustResolver
            .resolve(&refer("serde::Serialize"), "src/main.rs", &known)
            .kind,
        ResolutionKind::External
    );
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
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("crates/api/src/provider_error.rs"),
        "crate:: must resolve within the importing file's own crate"
    );
    Ok(())
}

#[test]
fn rust_crate_path_still_falls_back_to_top_level_src() -> TestResult {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz::qux"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/qux.rs"));
    Ok(())
}

#[test]
fn rust_glob_import_resolves_to_the_module_it_globs() -> TestResult {
    let known = paths(&["src/providers/mod.rs", "src/providers/openai.rs"]);
    let result = RustResolver.resolve(&refer("super::*"), "src/providers/openai.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("src/providers/mod.rs"),
        "a glob over the parent module must point at that module's file"
    );
    Ok(())
}

#[test]
fn rust_glob_import_resolves_against_a_named_module_too() -> TestResult {
    let known = paths(&["src/main.rs", "src/constants.rs"]);
    let result = RustResolver.resolve(&refer("super::constants::*"), "src/a/b.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/constants.rs"));
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
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("src/providers/shared.rs"),
        "super:: from a non-mod.rs file must mean its own parent module"
    );
    Ok(())
}

#[test]
fn rust_repeated_super_walks_up_one_module_per_repetition() -> TestResult {
    let known = paths(&["src/a/mod.rs", "src/a/b/mod.rs", "src/a/b/c.rs"]);
    let result = RustResolver.resolve(&refer("super::super::*"), "src/a/b/c.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/mod.rs"));
    Ok(())
}

#[test]
fn rust_super_from_a_mod_rs_file_still_resolves() -> TestResult {
    let known = paths(&["src/a/mod.rs", "src/thing.rs"]);
    let result = RustResolver.resolve(&refer("super::thing"), "src/a/mod.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/thing.rs"));
    Ok(())
}

#[test]
fn rust_self_resolves_against_the_files_own_module_dir() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/a/b/child.rs"]);
    let result = RustResolver.resolve(&refer("self::child"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/b/child.rs"));
    Ok(())
}

#[test]
fn rust_bare_crate_glob_resolves_to_the_crate_root_file() -> TestResult {
    let known = paths(&["Cargo.toml", "src/lib.rs", "src/a.rs"]);
    let result = RustResolver.resolve(&refer("crate::*"), "src/a.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/lib.rs"));
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
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("crates/api/src/lib.rs")
    );
    Ok(())
}

#[test]
fn rust_an_item_name_resolves_to_the_module_file_that_would_contain_it() -> TestResult {
    let known = paths(&["src/a/b.rs", "src/a/mod.rs"]);
    let result = RustResolver.resolve(&refer("super::nonexistent"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/mod.rs"));
    assert_eq!(result.confidence, Some("Low"));
    Ok(())
}

#[test]
fn rust_an_import_with_no_candidate_file_at_all_is_unresolved() -> TestResult {
    let known = paths(&["src/lonely.rs"]);
    let result = RustResolver.resolve(&refer("super::whatever"), "src/lonely.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
    Ok(())
}

#[test]
fn rust_crate_item_defined_in_the_crate_root_resolves_to_lib_rs() -> TestResult {
    let known = paths(&["src/lib.rs", "src/other.rs"]);
    let result = RustResolver.resolve(&refer("crate::Item"), "src/other.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(result.confidence, Some("Low"));
    Ok(())
}

#[test]
fn rust_crate_item_defined_in_main_rs_resolves_to_main_rs() -> TestResult {
    let known = paths(&["src/main.rs", "src/other.rs"]);
    let result = RustResolver.resolve(&refer("crate::Item"), "src/other.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/main.rs"));
    Ok(())
}

#[test]
fn rust_2018_path_starting_with_a_local_module_resolves_to_its_file() -> TestResult {
    let known = paths(&["src/main.rs", "src/cli.rs"]);
    let result = RustResolver.resolve(&refer("cli::Args"), "src/main.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/cli.rs"));
    assert_eq!(result.confidence, Some("Low"));
    Ok(())
}

#[test]
fn rust_2018_local_module_of_a_non_root_file_resolves_under_its_module_dir() -> TestResult {
    let known = paths(&["src/a.rs", "src/a/inner.rs"]);
    let result = RustResolver.resolve(&refer("inner::Thing"), "src/a.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/inner.rs"));
    Ok(())
}

#[test]
fn rust_pub_in_path_visibility_is_stripped_from_a_use_item() -> TestResult {
    let reference = RustResolver.extract_reference("pub(in crate::a) use crate::b::C;");
    assert_eq!(reference.map(|r| r.text), Some("crate::b::C".to_owned()));
    let plain = RustResolver.extract_reference("pub use crate::b;");
    assert_eq!(plain.map(|r| r.text), Some("crate::b".to_owned()));
    assert!(RustResolver.extract_reference("pubfn use x;").is_none());
    Ok(())
}
