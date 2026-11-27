//! Property-based tests for the import-resolver pipeline.
//!
//! These cover invariants that hand-written example tests can't reach:
//! patterns derived at random over a Vocabulary of synthetic import
//! statements, rather than carefully selected cases. Each proptest is a
//! fuzzer for a *round-trip* between two surface representations of an
//! import — usually between the raw text on the source line and the
//! canonical `ImportReference`.
//!
//! The covered round trip is surface→reference for Python-style
//! `from <M> import <S>`: regardless of whitespace
//! variation, `extract_reference` must produce `ImportReference{text:M}`
//! where `M` is the canonical (whitespace-stripped, original) module
//! path.
//!
//! [`proptest::proptest!`] supplies the strategy combinators, the
//! default 256-case runs, and the auto-shrinking on failure. Each test
//! is otherwise a pure function over the chosen Strategy.

use proptest::prelude::*;
use std::collections::HashSet;

use super::generic::{GenericResolver, LanguageResolver};
use super::normalize_join;
use crate::graph::rust_use_tree::parse_rust_import;

/// Vocabulary: a Python-style `from <M> import <S>` statement where
/// `<M>` is 1–3 dotted segments of alphanumeric+underscore
/// identifiers, and `<S>` is one identifier. Optional whitespace
/// around `from`, `<M>`, and `import` proves the parser's
/// whitespace-handling is robust.
///
/// Returns `(raw_text, expected_module, symbol)` so the proptest
/// can use `prop_assume!` to eliminate the
/// `<M> == <S>` degenerate case (where asserting "text equals
/// `<M>`" can't distinguish "extracted the right thing" from
/// "extracted a string-that-happens-to-equal-<M>").
fn python_from_vocab() -> impl Strategy<Value = (String, String, String)> {
    // Identifiers that look like Python — lowercase letter, then any
    // mix of letters/digits/underscores.
    let identifier = "[a-z][a-z0-9_]{0,7}";

    // The module path is 1–3 dotted segments. Bound at 3 because
    // proptest's case-count budget (256) covers modest module trees
    // well, and 4+ segments rarely surfaces parser bugs that 3 doesn't
    // already exercise.
    let segments = proptest::collection::vec(identifier, 1..=3);
    let symbol = identifier;

    // Whitespace variations: 1–2 spaces (or tabs) around each keyword
    // boundary. Lower bound is 1 because Python requires at least
    // one separator between `from`, `<M>`, `import`, and `<S>`. A lower bound
    // of 0 would generate inputs the parser is *supposed* to refuse
    // (`fromaimporta`), which we'd then assert on, conflating
    // "parser rejects" with "parser extract succeeded but produced
    // wrong text".
    let spaces_before = "[ \\t]{1,2}";
    let spaces_after_m = "[ \\t]{1,2}";
    let spaces_after_import = "[ \\t]{1,2}";

    (
        segments,
        symbol,
        spaces_before,
        spaces_after_m,
        spaces_after_import,
    )
        .prop_map(|(segs, sym, sb, sam, sai)| {
            let module = segs.join(".");
            // `from<M> import<spaces><S>` — note we vary whitespace
            // around `import`, between `<M>` and `import`, and before
            // `<M>`. We keep `import <S>` (the tail of the statement)
            // minimally varied because the parser falls through to
            // `split_whitespace().next()` there — testing it
            // differently doesn't yield new coverage.
            let raw = format!("from{sb}{module}{sam}import{sai}{sym}");
            (raw, module, sym)
        })
}

fn segment_vocab() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just(".".to_owned()),
        Just("..".to_owned()),
        Just("...".to_owned()),
        "[a-z0-9_]{1,6}",
    ]
}

proptest! {
    /// Round-trip #1 — Canonical Extraction under Whitespace Variation.
    ///
    /// For any Python-style `from <M> import <S>` statement in the
    /// Vocabulary — across whitespace variations around `from`,
    /// `<M>`, and `import` — the `extract_reference` pipeline must
    /// return `Some(ImportReference)` whose `text` equals the
    /// canonical (whitespace-stripped) module path `<M>`.
    ///
    /// Catches:
    /// - trimming inconsistencies
    /// - whitespace sensitivity around each keyword boundary
    /// - dotted-segment collapsing in `split_whitespace().next()`
    /// - parser consuming the wrong token (we rule out
    ///   `<M>`-equals-`<S>` cases via `prop_assume!` so a regression
    ///   that picked up `<S>` as the module path can't sneak through
    ///   just because both strings happen to be identical)
    #[test]
    fn python_from_import_extracts_canonical_module_path(
        (raw, expected_module, sym) in python_from_vocab()
    ) {
        prop_assume!(
            expected_module != sym,
            "skip degenerate case where module and symbol are identical \
             — it can't distinguish 'extracts the first token' from \
             'extracts any matching token'"
        );

        let resolver = GenericResolver::python();
        let extracted = resolver.extract_reference(&raw);
        let Some(reference) = extracted else {
            return Err(TestCaseError::fail(format!(
                "extract_reference unexpectedly returned None for raw={raw:?}"
            )));
        };
        // Equality on `&str` (via `as_str()`) so the macro doesn't move
        // `reference.text` or `expected_module` into its debug-format.
        // The `&str` form compares correctly and would still surface a
        // mismatch on failure (proptest's equality macro formats both
        // sides, and `&str` debug prints the string contents).
        //
        // Combined with `prop_assume!(expected_module != sym)` above,
        // this equality is enough coverage: if the parser extracted
        // `<S>` (the symbol) instead of `<M>` (the module path), the
        // strings differ and the assertion fails. A finer-grained
        // `ends_with(sym)` check would be a false positive: a valid
        // `a.b` module legitimately ends with the symbol string `b`,
        // so checking "doesn't end with sym" would reject correct parses
        // like `from a.b import b`.
        prop_assert_eq!(
            reference.text.as_str(),
            expected_module.as_str(),
            "extract produced wrong module path for raw={:?}",
            raw
        );
    }

    /// Property #2 — `parse_rust_import` never panics on arbitrary or malformed
    /// token soup and always returns bounded, deduplicated, non-empty targets.
    #[test]
    fn parse_rust_import_never_panics_and_preserves_bounds(
        raw in "([a-z0-9_:{},*;/# \\t\\n]|use|pub|crate|self|super|as|r#){0,120}"
    ) {
        if let Some(targets) = parse_rust_import(&raw) {
            if targets.len() > 256 {
                return Err(TestCaseError::fail(format!(
                    "expanded targets exceeded bound 256: {}",
                    targets.len()
                )));
            }
            let mut unique = HashSet::new();
            for target in &targets {
                if target.is_empty() {
                    return Err(TestCaseError::fail(
                        "parse_rust_import emitted an empty target".to_owned(),
                    ));
                }
                if !unique.insert(target.as_str()) {
                    return Err(TestCaseError::fail(format!(
                        "parse_rust_import emitted duplicate target {target:?}"
                    )));
                }
            }
        }
    }

    /// Property #3 — Canonical expansion of grouped Rust `use` trees across
    /// visibility, comments, aliases, `self`, and globs.
    #[test]
    fn parse_rust_import_expands_grouped_trees_canonically(
        vis in prop_oneof![Just(""), Just("pub "), Just("pub(crate) "), Just("pub(in crate::m) ")],
        root in prop_oneof![Just("crate"), Just("super"), Just("self"), Just("my_crate")],
        seg in "[a-z][a-z0-9_]{1,6}",
        leaf_a in "[a-z][a-z0-9_]{1,6}",
        leaf_b in "[a-z][a-z0-9_]{1,6}",
        alias in "[a-z][a-z0-9_]{1,6}",
        ws in "[ \\t]{0,2}",
    ) {
        prop_assume!(leaf_a != "self" && leaf_a != "as");
        prop_assume!(leaf_b != "self" && leaf_b != "as" && leaf_b != leaf_a);
        prop_assume!(alias != "as" && seg != "self" && seg != "as");

        let raw = format!(
            "{vis}use /* comment */ {root}::{seg}::{{{ws}self,{ws}{leaf_a} as {alias},{ws}{leaf_b}::*{ws}}};"
        );
        let Some(targets) = parse_rust_import(&raw) else {
            return Err(TestCaseError::fail(format!(
                "valid grouped Rust use returned None: {raw:?}"
            )));
        };
        let expected = vec![
            format!("{root}::{seg}"),
            format!("{root}::{seg}::{leaf_a}"),
            format!("{root}::{seg}::{leaf_b}::*"),
        ];
        prop_assert_eq!(targets, expected, "unexpected expansion for raw={:?}", raw);
    }

    /// Property #4 — `normalize_join` never panics on arbitrary segments and
    /// never escapes the project root or retains `.` / `..` / empty segments.
    #[test]
    fn normalize_join_never_panics_or_escapes_root(
        base_parts in proptest::collection::vec(segment_vocab(), 0..=6),
        rel_parts in proptest::collection::vec(segment_vocab(), 0..=6),
    ) {
        let base_dir = base_parts.join("/");
        let relative = rel_parts.join("/");

        let mut expected_depth: Option<usize> = Some(0);
        for part in base_dir.split('/').chain(relative.split('/')) {
            let Some(cur) = expected_depth else {
                break;
            };
            match part {
                "" | "." => {}
                ".." => {
                    expected_depth = cur.checked_sub(1);
                }
                _ => {
                    expected_depth = cur.checked_add(1);
                }
            }
        }

        let actual = normalize_join(&base_dir, &relative);
        match (expected_depth, actual) {
            (None, Some(escaped)) => {
                return Err(TestCaseError::fail(format!(
                    "normalize_join escaped root instead of returning None: {escaped:?}"
                )));
            }
            (Some(_), None) => {
                return Err(TestCaseError::fail(format!(
                    "normalize_join unexpectedly returned None for base={base_dir:?} rel={relative:?}"
                )));
            }
            (None, None) => {}
            (Some(depth), Some(joined)) => {
                if joined.starts_with('/') || joined.ends_with('/') || joined.contains("//") {
                    return Err(TestCaseError::fail(format!(
                        "unnormalized slashes in {joined:?}"
                    )));
                }
                let segments: Vec<&str> = if joined.is_empty() {
                    Vec::new()
                } else {
                    joined.split('/').collect()
                };
                prop_assert_eq!(segments.len(), depth);
                for seg in segments {
                    if seg.is_empty() || seg == "." || seg == ".." {
                        return Err(TestCaseError::fail(format!(
                            "unnormalized segment {seg:?} in {joined:?}"
                        )));
                    }
                }
            }
        }
    }
}
