// slugaudit-line-exception: approved-by=agent; reason=one test per Rust import form (workspace, glob, super/self, item names); each is small and independently named
use super::*;
use crate::graph::reference::ImportReference;
use crate::graph::resolver::ResolutionKind;
use std::collections::HashSet;

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

fn paths<'a>(list: &[&'a str]) -> HashSet<&'a str> {
    list.iter().copied().collect()
}

#[test]
fn rust_crate_path_resolves_under_src() {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz::qux"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/qux.rs"));
    assert_eq!(result.confidence, Some("High"));
}

#[test]
fn rust_crate_path_resolves_mod_rs_form() {
    let known = paths(&["src/main.rs", "src/baz/mod.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/mod.rs"));
}

#[test]
fn rust_crate_path_with_a_trailing_item_name_resolves_to_its_module_file() {
    let known = paths(&["src/main.rs", "src/helper.rs"]);
    let result = RustResolver.resolve(&refer("crate::helper::greet"), "src/main.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/helper.rs"));
}

#[test]
fn rust_crate_path_prefers_the_longest_directory_chain_that_actually_exists() {
    let known = paths(&["src/a.rs", "src/a/b.rs"]);
    let result = RustResolver.resolve(&refer("crate::a::b"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/b.rs"));
}

#[test]
fn rust_super_is_a_low_confidence_heuristic() {
    let known = paths(&["src/a/b.rs", "src/thing.rs"]);
    let result = RustResolver.resolve(&refer("super::thing"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.confidence, Some("Low"));
    assert_eq!(result.to_relative_path.as_deref(), Some("src/thing.rs"));
}

#[test]
fn rust_self_is_a_low_confidence_heuristic() {
    let known = paths(&["src/a/b.rs", "src/a/sibling.rs"]);
    let result = RustResolver.resolve(&refer("self::sibling"), "src/a/b.rs", &known);
    assert_eq!(result.confidence, Some("Low"));
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/sibling.rs"));
}

#[test]
fn rust_std_and_external_crates_are_external() {
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
}

#[test]
fn rust_crate_path_is_anchored_at_the_owning_crate_in_a_workspace() {
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
}

#[test]
fn rust_crate_path_still_falls_back_to_top_level_src() {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = RustResolver.resolve(&refer("crate::baz::qux"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/qux.rs"));
}

#[test]
fn rust_glob_import_resolves_to_the_module_it_globs() {
    let known = paths(&["src/providers/mod.rs", "src/providers/openai.rs"]);
    let result = RustResolver.resolve(&refer("super::*"), "src/providers/openai.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("src/providers/mod.rs"),
        "a glob over the parent module must point at that module's file"
    );
}

#[test]
fn rust_glob_import_resolves_against_a_named_module_too() {
    let known = paths(&["src/main.rs", "src/constants.rs"]);
    let result = RustResolver.resolve(&refer("super::constants::*"), "src/a/b.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/constants.rs"));
}

#[test]
fn rust_super_resolves_against_the_real_module_tree_not_the_grandparent_dir() {
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
}

#[test]
fn rust_repeated_super_walks_up_one_module_per_repetition() {
    let known = paths(&["src/a/mod.rs", "src/a/b/mod.rs", "src/a/b/c.rs"]);
    let result = RustResolver.resolve(&refer("super::super::*"), "src/a/b/c.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/mod.rs"));
}

#[test]
fn rust_super_from_a_mod_rs_file_still_resolves() {
    let known = paths(&["src/a/mod.rs", "src/thing.rs"]);
    let result = RustResolver.resolve(&refer("super::thing"), "src/a/mod.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/thing.rs"));
}

#[test]
fn rust_self_resolves_against_the_files_own_module_dir() {
    let known = paths(&["src/a/b.rs", "src/a/b/child.rs"]);
    let result = RustResolver.resolve(&refer("self::child"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/b/child.rs"));
}

#[test]
fn rust_bare_crate_glob_resolves_to_the_crate_root_file() {
    let known = paths(&["Cargo.toml", "src/lib.rs", "src/a.rs"]);
    let result = RustResolver.resolve(&refer("crate::*"), "src/a.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/lib.rs"));
}

#[test]
fn rust_super_glob_from_a_top_level_module_resolves_to_the_crate_root() {
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
}

#[test]
fn rust_an_item_name_resolves_to_the_module_file_that_would_contain_it() {
    let known = paths(&["src/a/b.rs", "src/a/mod.rs"]);
    let result = RustResolver.resolve(&refer("super::nonexistent"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/mod.rs"));
    assert_eq!(result.confidence, Some("Low"));
}

#[test]
fn rust_an_import_with_no_candidate_file_at_all_is_unresolved() {
    let known = paths(&["src/lonely.rs"]);
    let result = RustResolver.resolve(&refer("super::whatever"), "src/lonely.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
}
