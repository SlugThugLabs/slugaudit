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

/// A Cargo workspace puts each crate under its own directory, so `crate::`
/// is anchored at *that crate's* `src`, not a single top-level `src`.
/// Assuming a top-level `src` made every `crate::` import in every non-root
/// crate of a workspace unresolvable — 552 edges in one real project.
/// `crate::` in a plain single-crate project (no workspace) keeps working
/// against the top-level `src` even when no manifest happens to be indexed.
/// `use super::*;` is the single most common import form in real Rust code
/// (257 occurrences in one project). It globs the *parent module*, so the
/// edge points at that module's own file — previously the resolver searched
/// for a file literally named `*.rs` and never resolved any of them.
/// `super::` from `src/providers/openai.rs` means module `providers`, whose
/// file is `src/providers/mod.rs` — not `src/`, which is what the old
/// `parent_dir(parent_dir(..))` heuristic produced. That heuristic was only
/// correct when the importing file was itself a `mod.rs`.
/// `super::super::*` walks up two modules, not one.
/// The strict module-tree interpretation is tried first, but a genuinely
/// `mod.rs`-shaped tree must still resolve through the older heuristic.
/// `self::` means the importing file's *own* module, so a child module of
/// `src/a/b.rs` lives under `src/a/b/`.
/// A `crate::`-rooted glob points at the crate root file itself.
/// `use super::*;` in a *top-level* module (`src/foo.rs`) refers to the
/// crate root, whose file is `lib.rs`/`main.rs` — there is no `src.rs` or
/// `src/mod.rs` to find.
/// `super::nonexistent` names something in the parent module — whether or
/// not that item exists, the *file* it would live in is the parent
/// module's own file. SlugAudit indexes files, not items, so this
/// correctly resolves to `src/a/mod.rs` (at `"Low"` confidence, since we
/// cannot prove the item is really there). The edge is only Unresolved
/// when no candidate file exists at all — see
/// `rust_an_import_with_no_candidate_file_at_all_is_unresolved`.
/// When neither a segment chain nor the base module's own file exists,
/// the edge must stay Unresolved rather than being forced onto some
/// unrelated nearby file.
#[test]
fn an_unsupported_language_is_always_unresolved() {
    let known = paths(&["a.go"]);
    let result = resolve("go", &refer("./local"), "a.go", &known);
    assert_eq!(result.kind, ResolutionKind::Unresolved);
}
