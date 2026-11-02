//! Tests for the token gate on the JS branch of
//! `GenericResolver::extract_reference`.
//!
//! The gate matches `from` as a keyword (delimited by whitespace), not as a
//! substring anywhere in the line. A substring gate would route the line
//! through the JS/TS extractor, which returns `None` when it finds no quoted
//! string after `from`. That is the right verdict for the malformed case
//! `import x from broken` (no quoted path), but the wrong verdict for any line
//! whose MODULE NAME contains `from` as a substring: `import from_util`,
//! `import foo_bar.from_baz`, etc. Those are legal Python bare-module imports
//! and must reach the generic `import X` extractor below — they resolve like
//! any other bare module name, not silently dropped.
//!
//! These tests pin both halves: lines with `from` inside a module name
//! extract, and real JS-shaped lines still do.
use super::generic::{GenericResolver, GenericResolverConfig, LanguageResolver};
use slugaudit_test_check::{Check, TestResult};
use std::collections::HashSet;

fn paths<'a>(set: &'a [&'a str]) -> HashSet<&'a str> {
    set.iter().copied().collect()
}

#[test]
fn python_bare_module_named_from_util_is_not_dropped() -> TestResult {
    // `import from_util` is a legal Python bare-module import. Token-gating
    // routes it to the generic `import X` extractor and the bare-name branch
    // returns `from_util` as a bare module name (a substring-gated JS branch
    // would drop it).
    let resolver = GenericResolver::python();
    let reference = resolver
        .extract_reference("import from_util")
        .check("import from_util must not be dropped")?;
    assert_eq!(reference.text, "from_util");
    Ok(())
}

#[test]
fn python_dotted_module_with_from_substring_is_not_dropped() -> TestResult {
    // `import foo_bar.from_baz`: `from` appears as a substring of the module
    // path. The module name is legal Python (`foo_bar.from_baz` is a real import
    // path pattern), so it must not be dropped.
    let resolver = GenericResolver::python();
    let reference = resolver
        .extract_reference("import foo_bar.from_baz")
        .check("import foo_bar.from_baz must not be dropped")?;
    assert_eq!(reference.text, "foo_bar.from_baz");
    Ok(())
}

#[test]
fn dotted_module_with_from_substring_resolves_to_real_file() -> TestResult {
    // End-to-end through the catch-all (non-Python) generic resolver
    // with `bare_names_are_external = false`, so the dotted path goes
    // through module-path resolution rather than the bare-name shortcut.
    // The point is: not dropped. Resolves via module-path extension
    // lookup against `known_paths`.
    let resolver = GenericResolver::new(GenericResolverConfig {
        bare_names_are_external: false,
        ..GenericResolverConfig::default()
    });
    let known = paths(&["foo_bar/from_baz.py"]);
    let reference = resolver
        .extract_reference("import foo_bar.from_baz")
        .check("extract succeeds")?;
    let resolution = resolver.resolve(&reference, "main.py", &known);
    assert_eq!(resolution.kind.as_sql_text(), "Resolved");
    assert_eq!(
        resolution.to_relative_path.as_deref(),
        Some("foo_bar/from_baz.py"),
    );
    Ok(())
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
fn valid_js_import_still_extracts_the_quoted_path() -> TestResult {
    // The case the JS branch exists for:
    // `import x from 'foo.js'` → `text: "foo.js"`.
    let resolver = GenericResolver::js();
    let reference = resolver
        .extract_reference("import x from 'foo.js'")
        .check("valid JS import extracts cleanly")?;
    assert_eq!(reference.text, "foo.js");
    Ok(())
}

#[test]
fn from_substring_inside_quoted_path_is_not_a_keyword() -> TestResult {
    // A path string that itself contains the word "from" (e.g.
    // a generated module name like `meta/from-handler`) is fine —
    // `split_whitespace` treats the whole quoted path as one token
    // because it doesn't have embedded whitespace, so the token gate
    // sees only the keyword `from` outside the quotes.
    let resolver = GenericResolver::js();
    let reference = resolver
        .extract_reference("import x from 'meta/from-handler'")
        .check("valid path with the word 'from' inside it")?;
    assert_eq!(reference.text, "meta/from-handler");
    Ok(())
}
