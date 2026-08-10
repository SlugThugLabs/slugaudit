//! Property-based tests for the import-resolver pipeline.
//!
//! These cover invariants that hand-written example tests can't reach:
//! patterns derived at random over a Vocabulary of synthetic import
//! statements, rather than carefully selected cases. Each proptest is a
//! fuzzer for a *round-trip* between two surface representations of an
//! import — usually between the raw text on the source line and the
//! canonical `ImportReference` (and, in later rounds, between the
//! reference and the resolved `Resolution`).
//!
//! Foundation Step 1 covers the surface→reference round trip for
//! Python-style `from <M> import <S>`: regardless of whitespace
//! variation, `extract_reference` must produce `ImportReference{text:M}`
//! where `M` is the canonical (whitespace-stripped, original) module
//! path. Future rounds add resolve→candidate round trips and JS/TS
//! quote-variant equivalence.
//!
//! [`proptest::proptest!`] supplies the strategy combinators, the
//! default 256-case runs, and the auto-shrinking on failure. Each test
//! is otherwise a pure function over the chosen Strategy.

use proptest::prelude::*;

use super::generic::{GenericResolver, LanguageResolver};

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
    // one separator between `from`, `<M>`, `import`, and `<S>`. (The
    // proptest caught this on the first run — the `fromaimporta`
    // rejection — when the lower bound was 0 and the parser correctly
    // refused it. Lower bounds of 0 generate inputs the parser is
    // *supposed* to refuse, which we'd then assert on, conflating
    // "parser rejects" with "parser extract succeeded but produced
    // wrong text".)
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
        prop_assert!(
            extracted.is_some(),
            "extract_reference unexpectedly returned None for raw={:?}",
            raw
        );
        let reference = extracted.unwrap();
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
        // `ends_with(sym)` check turned out false-positive: a valid
        // `a.b` module legitimately ends with the symbol string `b`,
        // so checking "doesn't end with sym" rejected correct parses
        // like `from a.b import b`.
        prop_assert_eq!(
            reference.text.as_str(),
            expected_module.as_str(),
            "extract produced wrong module path for raw={:?}",
            raw
        );
    }
}
