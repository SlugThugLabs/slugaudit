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

#[test]
fn an_unsupported_language_is_always_unresolved() {
    let known = paths(&["a.go"]);
    let result = resolve("go", &refer("./local"), "a.go", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
}
