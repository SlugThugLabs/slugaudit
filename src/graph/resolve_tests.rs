use super::*;

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

fn paths<'a>(list: &[&'a str]) -> HashSet<&'a str> {
    list.iter().copied().collect()
}

#[test]
fn python_dot_import_resolves_to_package_init() {
    let known = paths(&["pkg/__init__.py", "pkg/a.py"]);
    let result = resolve("python", &refer("."), "pkg/a.py", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("pkg/__init__.py"));
}

#[test]
fn python_single_dot_sibling_module() {
    let known = paths(&["pkg/a.py", "pkg/bar.py"]);
    let result = resolve("python", &refer(".bar"), "pkg/a.py", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("pkg/bar.py"));
}

#[test]
fn python_double_dot_goes_up_one_package() {
    let known = paths(&["pkg/sub/a.py", "pkg/mod.py"]);
    let result = resolve("python", &refer("..mod"), "pkg/sub/a.py", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("pkg/mod.py"));
}

#[test]
fn python_dotted_path_prefers_package_form() {
    let known = paths(&["pkg/a.py", "pkg/sub/__init__.py"]);
    let result = resolve("python", &refer(".sub"), "pkg/a.py", &known);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("pkg/sub/__init__.py")
    );
}

#[test]
fn python_absolute_import_is_external() {
    let known = paths(&["pkg/a.py"]);
    let result = resolve("python", &refer("os"), "pkg/a.py", &known);
    assert_eq!(result.kind, ResolutionKind::External);
    assert_eq!(result.to_relative_path, None);
}

#[test]
fn python_relative_import_with_no_matching_file_is_unresolved() {
    let known = paths(&["pkg/a.py"]);
    let result = resolve("python", &refer(".missing"), "pkg/a.py", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
}

#[test]
fn js_relative_import_resolves_with_guessed_extension() {
    let known = paths(&["src/a.ts", "src/utils.ts"]);
    let result = resolve("javascript", &refer("./utils"), "src/a.ts", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/utils.ts"));
    assert_eq!(result.confidence, Some("High"));
}

#[test]
fn js_relative_import_resolves_to_directory_index() {
    let known = paths(&["src/a.ts", "src/lib/index.ts"]);
    let result = resolve("javascript", &refer("../lib"), "src/pages/a.ts", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/lib/index.ts"));
}

#[test]
fn js_ambiguous_extension_match_is_low_confidence() {
    let known = paths(&["src/a.ts", "src/utils.ts", "src/utils.js"]);
    let result = resolve("javascript", &refer("./utils"), "src/a.ts", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.confidence, Some("Low"));
}

#[test]
fn js_bare_package_name_is_external() {
    let known = paths(&["src/a.ts"]);
    let result = resolve("javascript", &refer("react"), "src/a.ts", &known);
    assert_eq!(result.kind, ResolutionKind::External);
}

#[test]
fn rust_crate_path_resolves_under_src() {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = resolve("rust", &refer("crate::baz::qux"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/qux.rs"));
    assert_eq!(result.confidence, Some("High"));
}

#[test]
fn rust_crate_path_resolves_mod_rs_form() {
    let known = paths(&["src/main.rs", "src/baz/mod.rs"]);
    let result = resolve("rust", &refer("crate::baz"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/mod.rs"));
}

#[test]
fn rust_crate_path_with_a_trailing_item_name_resolves_to_its_module_file() {
    // `use crate::helper::greet;` imports the function `greet`, not a
    // submodule — `greet` must be dropped as an item name so this
    // resolves to `src/helper.rs`, not `src/helper/greet.rs`.
    let known = paths(&["src/main.rs", "src/helper.rs"]);
    let result = resolve(
        "rust",
        &refer("crate::helper::greet"),
        "src/main.rs",
        &known,
    );
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/helper.rs"));
}

#[test]
fn rust_crate_path_prefers_the_longest_directory_chain_that_actually_exists() {
    // Both `src/a/b.rs` (b as a module) and `src/a.rs` (b as an item in a)
    // could satisfy `crate::a::b` — the longer, more specific interpretation
    // wins when it's the one that's real.
    let known = paths(&["src/a.rs", "src/a/b.rs"]);
    let result = resolve("rust", &refer("crate::a::b"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/b.rs"));
}

#[test]
fn rust_super_is_a_low_confidence_heuristic() {
    let known = paths(&["src/a/b.rs", "src/thing.rs"]);
    let result = resolve("rust", &refer("super::thing"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.confidence, Some("Low"));
    assert_eq!(result.to_relative_path.as_deref(), Some("src/thing.rs"));
}

#[test]
fn rust_self_is_a_low_confidence_heuristic() {
    let known = paths(&["src/a/b.rs", "src/a/sibling.rs"]);
    let result = resolve("rust", &refer("self::sibling"), "src/a/b.rs", &known);
    assert_eq!(result.confidence, Some("Low"));
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/sibling.rs"));
}

#[test]
fn rust_std_and_external_crates_are_external() {
    let known = paths(&["src/main.rs"]);
    assert_eq!(
        resolve(
            "rust",
            &refer("std::collections::HashMap"),
            "src/main.rs",
            &known
        )
        .kind,
        ResolutionKind::External
    );
    assert_eq!(
        resolve("rust", &refer("serde::Serialize"), "src/main.rs", &known).kind,
        ResolutionKind::External
    );
}

/// A Cargo workspace puts each crate under its own directory, so `crate::`
/// is anchored at *that crate's* `src`, not a single top-level `src`.
/// Assuming a top-level `src` made every `crate::` import in every non-root
/// crate of a workspace unresolvable — 552 edges in one real project.
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
    let result = resolve(
        "rust",
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

/// `crate::` in a plain single-crate project (no workspace) keeps working
/// against the top-level `src` even when no manifest happens to be indexed.
#[test]
fn rust_crate_path_still_falls_back_to_top_level_src() {
    let known = paths(&["src/main.rs", "src/baz/qux.rs"]);
    let result = resolve("rust", &refer("crate::baz::qux"), "src/main.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/baz/qux.rs"));
}

/// `use super::*;` is the single most common import form in real Rust code
/// (257 occurrences in one project). It globs the *parent module*, so the
/// edge points at that module's own file — previously the resolver searched
/// for a file literally named `*.rs` and never resolved any of them.
#[test]
fn rust_glob_import_resolves_to_the_module_it_globs() {
    let known = paths(&["src/providers/mod.rs", "src/providers/openai.rs"]);
    let result = resolve(
        "rust",
        &refer("super::*"),
        "src/providers/openai.rs",
        &known,
    );
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
    let result = resolve("rust", &refer("super::constants::*"), "src/a/b.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/constants.rs"));
}

/// `super::` from `src/providers/openai.rs` means module `providers`, whose
/// file is `src/providers/mod.rs` — not `src/`, which is what the old
/// `parent_dir(parent_dir(..))` heuristic produced. That heuristic was only
/// correct when the importing file was itself a `mod.rs`.
#[test]
fn rust_super_resolves_against_the_real_module_tree_not_the_grandparent_dir() {
    let known = paths(&[
        "src/providers/mod.rs",
        "src/providers/openai.rs",
        "src/providers/shared.rs",
    ]);
    let result = resolve(
        "rust",
        &refer("super::shared"),
        "src/providers/openai.rs",
        &known,
    );
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("src/providers/shared.rs"),
        "super:: from a non-mod.rs file must mean its own parent module"
    );
}

/// `super::super::*` walks up two modules, not one.
#[test]
fn rust_repeated_super_walks_up_one_module_per_repetition() {
    let known = paths(&["src/a/mod.rs", "src/a/b/mod.rs", "src/a/b/c.rs"]);
    let result = resolve("rust", &refer("super::super::*"), "src/a/b/c.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/mod.rs"));
}

/// The strict module-tree interpretation is tried first, but a genuinely
/// `mod.rs`-shaped tree must still resolve through the older heuristic.
#[test]
fn rust_super_from_a_mod_rs_file_still_resolves() {
    let known = paths(&["src/a/mod.rs", "src/thing.rs"]);
    let result = resolve("rust", &refer("super::thing"), "src/a/mod.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/thing.rs"));
}

/// `self::` means the importing file's *own* module, so a child module of
/// `src/a/b.rs` lives under `src/a/b/`.
#[test]
fn rust_self_resolves_against_the_files_own_module_dir() {
    let known = paths(&["src/a/b.rs", "src/a/b/child.rs"]);
    let result = resolve("rust", &refer("self::child"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/b/child.rs"));
}

/// A `crate::`-rooted glob points at the crate root file itself.
#[test]
fn rust_bare_crate_glob_resolves_to_the_crate_root_file() {
    let known = paths(&["Cargo.toml", "src/lib.rs", "src/a.rs"]);
    let result = resolve("rust", &refer("crate::*"), "src/a.rs", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/lib.rs"));
}

/// `use super::*;` in a *top-level* module (`src/foo.rs`) refers to the
/// crate root, whose file is `lib.rs`/`main.rs` — there is no `src.rs` or
/// `src/mod.rs` to find.
#[test]
fn rust_super_glob_from_a_top_level_module_resolves_to_the_crate_root() {
    let known = paths(&[
        "crates/api/Cargo.toml",
        "crates/api/src/lib.rs",
        "crates/api/src/codex_adapter.rs",
    ]);
    let result = resolve(
        "rust",
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

/// `super::nonexistent` names something in the parent module — whether or
/// not that item exists, the *file* it would live in is the parent
/// module's own file. SlugAudit indexes files, not items, so this
/// correctly resolves to `src/a/mod.rs` (at `"Low"` confidence, since we
/// cannot prove the item is really there). The edge is only Unresolved
/// when no candidate file exists at all — see
/// `rust_an_import_with_no_candidate_file_at_all_is_unresolved`.
#[test]
fn rust_an_item_name_resolves_to_the_module_file_that_would_contain_it() {
    let known = paths(&["src/a/b.rs", "src/a/mod.rs"]);
    let result = resolve("rust", &refer("super::nonexistent"), "src/a/b.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Resolved);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/a/mod.rs"));
    assert_eq!(result.confidence, Some("Low"));
}

/// When neither a segment chain nor the base module's own file exists,
/// the edge must stay Unresolved rather than being forced onto some
/// unrelated nearby file.
#[test]
fn rust_an_import_with_no_candidate_file_at_all_is_unresolved() {
    let known = paths(&["src/lonely.rs"]);
    let result = resolve("rust", &refer("super::whatever"), "src/lonely.rs", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
}

#[test]
fn an_unsupported_language_is_always_unresolved() {
    let known = paths(&["a.go"]);
    let result = resolve("go", &refer("./local"), "a.go", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
}
