//! Regression tests for the `contains("from")` substring gate that
//! lived in `GenericResolver::extract_reference` before the C1 fix.
//!
//! The prior implementation matched the substring `"from"` anywhere
//! in the line and routed the line through the JS/TS extractor,
//! returning `None` when the JS extractor couldn't find a quoted
//! string after `from`. That's the right verdict for the malformed
//! case `import x from broken` (no quoted path), but it's the wrong
//! verdict for any line whose MODULE NAME contains `from` as a
//! substring: `import from_util`, `import foo_bar.from_baz`, etc.
//! Those are legal Python bare-module imports and must reach the
//! generic `import X` extractor below — they should resolve
//! like any other bare module name, not be silently dropped.
//!
//! The fix token-gates the JS branch on `from` being a real keyword
//! (delimited by whitespace), not a substring. These tests pin both
//! halves of that correction: the previously-dropped cases now
//! extract, and the previously-handled cases still do.
use super::generic::{GenericResolver, GenericResolverConfig, LanguageResolver};
use std::collections::HashSet;

fn paths<'a>(set: &'a [&'a str]) -> HashSet<&'a str> {
    set.iter().copied().collect()
}

#[test]
fn python_bare_module_named_from_util_is_not_dropped() {
    // The bug: `import from_util` was dropped by the substring-gated
    // JS branch even though it's a legal Python bare-module import.
    // The fix: token-gating routes it to the generic `import X`
    // extractor and the bare-name branch returns `from_util` as a
    // bare module name.
    let resolver = GenericResolver::python();
    let reference = resolver
        .extract_reference("import from_util")
        .expect("import from_util must not be dropped");
    assert_eq!(reference.text, "from_util");
}

#[test]
fn python_dotted_module_with_from_substring_is_not_dropped() {
    // The bug: `import foo_bar.from_baz` was dropped because `from`
    // appeared as a substring of the module path. The module name is
    // legal Python (`foo_bar.from_baz` is a real import path
    // pattern).
    let resolver = GenericResolver::python();
    let reference = resolver
        .extract_reference("import foo_bar.from_baz")
        .expect("import foo_bar.from_baz must not be dropped");
    assert_eq!(reference.text, "foo_bar.from_baz");
}

#[test]
fn dotted_module_with_from_substring_resolves_to_real_file() {
    // End-to-end through the catch-all (non-Python) generic resolver
    // where `bare_names_are_external = false` (well, it is true by
    // default; we use a custom config so the dotted path goes through
    // module-path resolution rather than the bare-name shortcut).
    // The point is: not dropped. Resolves via module-path extension
    // lookup against `known_paths`.
    let resolver = GenericResolver::new(GenericResolverConfig {
        bare_names_are_external: false,
        ..GenericResolverConfig::default()
    });
    let known = paths(&["foo_bar/from_baz.py"]);
    let reference = resolver
        .extract_reference("import foo_bar.from_baz")
        .expect("extract succeeds");
    let resolution = resolver.resolve(&reference, "main.py", &known);
    assert_eq!(resolution.kind.as_sql_text(), "Resolved");
    assert_eq!(
        resolution.to_relative_path.as_deref(),
        Some("foo_bar/from_baz.py"),
    );
}

#[test]
fn malformed_js_import_with_unquoted_from_still_drops() {
    // Token-gating keeps the JS branch's behavioural purpose intact:
    // a real JS-shape with `from` as a keyword but no quoted path is
    // malformed and must keep returning `None`. Falling through
    // would let the generic `import X` extractor lift `x` out as a
    // bare module name and call it External, silently mislabeling
    // unresolved syntax as a third-party import.
    let resolver = GenericResolver::js();
    assert!(
        resolver.extract_reference("import x from broken").is_none(),
        "malformed JS `from` without a quote must still drop under JS",
    );
}

#[test]
fn valid_js_import_still_extracts_the_quoted_path() {
    // Regression guard for the case the JS branch was originally
    // handling: `import x from 'foo.js'` → `text: "foo.js"`.
    let resolver = GenericResolver::js();
    let reference = resolver
        .extract_reference("import x from 'foo.js'")
        .expect("valid JS import extracts cleanly");
    assert_eq!(reference.text, "foo.js");
}

#[test]
fn from_substring_inside_quoted_path_is_not_a_keyword() {
    // A path string that itself contains the word "from" (e.g.
    // a generated module name like `meta/from-handler`) is fine —
    // `split_whitespace` treats the whole quoted path as one token
    // because it doesn't have embedded whitespace, so the token gate
    // sees only the keyword `from` outside the quotes.
    let resolver = GenericResolver::js();
    let reference = resolver
        .extract_reference("import x from 'meta/from-handler'")
        .expect("valid path with the word 'from' inside it");
    assert_eq!(reference.text, "meta/from-handler");
}
