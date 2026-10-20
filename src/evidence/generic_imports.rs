//! Generic import extraction for languages the pack's own import pass
//! doesn't cover.
//!
//! The pack's `process()` only extracts imports for a hardcoded handful of
//! languages (python, js/ts, rust, go, java/kotlin, elixir). For the other
//! ~365 languages it returns no `ImportInfo` at all, so no dependency edges
//! ever existed for them — even though their grammars name import nodes
//! recognizably (`import_declaration` in swift/scala, `import_header` in
//! kotlin, `using_directive` in csharp, `preproc_include` in c/cpp,
//! `import_specification` in dart, etc.).
//!
//! This module walks the parse tree with generic node-kind patterns that
//! are consistent across tree-sitter grammars — the same mechanism as the
//! variable-binding walker in `normalize` — and emits `Import` evidence for
//! any node whose kind denotes a whole import statement. It runs only when
//! the pack produced zero imports for a file, so pack-covered languages
//! never double-report and never pay an extra parse.
//!
//! Node kinds are matched by *name* rather than per-language query; the
//! resulting `source` text is the raw statement sliced from the original
//! source, exactly like the pack's own `ImportInfo.source`.

use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin};
use serde_json::json;
use tree_sitter_language_pack::Node;
#[cfg(test)]
use tree_sitter_language_pack::get_parser;

/// Node kinds that denote a whole import statement. These names are shared
/// across tree-sitter grammars — `import_declaration` in swift and scala,
/// `import_statement` in julia and python, `preproc_include` in c/cpp/cuda/
/// glsl. Matching on the name rather than a per-language query is what
/// keeps this generic. Container kinds whose children are the real
/// imports (e.g. go's `import_declaration` wrapping `import_spec`s) are
/// harmless here because go is pack-covered: the walker is gated on the
/// pack finding zero imports, and a go file with zero pack imports has no
/// `import_declaration` nodes either. If a future language needs
/// child-level import nodes, this list is the single place to extend.
///
/// Exact match only (C7). The bare `"import"` kind is kept because
/// Haskell's grammar names its import statement `import` (verified by the
/// `haskell_import_is_captured` matrix test — C7's "prove a grammar needs
/// it" carve-out); the matrix's no-imports test guarantees a non-import
/// `import`-kind node doesn't appear in ordinary function/expression
/// source. Every other kind is verified by the grammar-matrix tests —
/// add a kind only with a regression test proving a real grammar needs it.
fn is_import_statement_kind(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement"
            | "import_from_statement"
            | "import_declaration"
            | "import_header"
            | "using_directive"
            | "using_statement"
            | "import_directive"
            | "import_specification"
            | "library_import"
            | "use_statement"
            | "namespace_use_declaration"
            | "preproc_include"
            | "open_module"
            | "pp_include_lib"
            | "import"
    )
}

/// Runs the generic import walker. Returns empty when the language has no
/// parser or the source doesn't parse — callers treat that as \\"no imports
/// found\\", not an error, matching `extract_bindings`'s contract.
///
/// Production code uses [`super::normalize::extract_extra_walkers`] (which
/// shares a tree with the binding walker); this standalone wrapper exists
/// for tests that want to assert the walker output for a single language
/// without going through the full evidence pipeline, so it is compiled
/// only under `cfg(test)` — production never links it.
#[cfg(test)]
pub(super) fn extract_generic_imports(language: &str, source: &str) -> Vec<EvidenceItem> {
    let Ok(mut parser) = get_parser(language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(source) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut counter = 0usize;
    let lines = crate::model::LineIndex::new(source);
    walk_imports(&tree.root_node(), source, &lines, &mut items, &mut counter);
    items
}

/// Walk emitting one `Import` evidence item per import-shaped
/// node, slicing the raw statement text from `source` via byte range.
/// Child nodes of a matched statement are still visited (a `using` block
/// may contain nested directives), but only whole-statement kinds emit.
///
/// A matched node whose **parent** is also an import-shaped node is
/// skipped: some grammars nest the real import inside a wrapper that is
/// itself import-shaped (dart: `import_or_export → library_import →
/// `import_specification`; haskell: `imports → import`). Emitting at
/// both levels would double-report the same statement (a C7
/// grammar-matrix finding). The outermost matched node owns the emit;
/// nested matches are containers, not statements.
///
/// `pub(super)` so `normalize.rs::extract_extra_walkers` can run this
/// walker on the same already-parsed tree it uses for the
/// variable-binding walker — saving the redundant second parse that
/// the C2 audit flagged.
///
/// The walk is a loop over an explicit heap stack, never recursion (see
/// [`super::push_children`]): each entry carries whether it is *suppressed*
/// — an import-shaped node directly under another import-shaped node, which
/// is walked past without emitting.
pub(super) fn walk_imports(
    root: &Node,
    source: &str,
    lines: &crate::model::LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) -> bool {
    let mut capped = false;
    let mut pending = vec![(root.clone(), 0, false)];
    while let Some((node, depth, suppressed)) = pending.pop() {
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            capped = true;
            continue;
        }
        let is_import = is_import_statement_kind(&node.kind());
        if is_import && !suppressed {
            // Trim: some grammars (e.g. c/cpp `preproc_include`) include the
            // trailing newline in the statement node's byte range.
            match super::node_text(source, node.start_byte(), node.end_byte()) {
                Ok(text) => push_import(text.trim(), &node, lines, items, counter),
                Err(error) => {
                    tracing::warn!(%error, "import statement skipped; extraction is partial");
                    capped = true;
                }
            }
        }
        // An import-shaped child of an import-shaped node (emitted or itself
        // suppressed) is a nested container: walk past it for further
        // nesting, but don't re-emit the same statement. Guarded by the
        // child's own kind so a non-import child (e.g. a string literal) is
        // never skipped.
        super::push_children(&mut pending, &node, |child| {
            let nested = is_import && is_import_statement_kind(&child.kind());
            (child, depth + 1, nested)
        });
    }
    capped
}

fn push_import(
    text: &str,
    node: &Node,
    lines: &crate::model::LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) {
    let key = format!("generic_import:{counter}");
    *counter += 1;
    items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::GenericWalker,
        span: super::node_span(node, lines, "generic import"),
        payload: json!({
            "source": text,
            "items": [],
            "alias": null,
            "is_wildcard": text.contains('*'),
        }),
    });
}

#[cfg(test)]
#[path = "generic_imports_tests.rs"]
mod tests;
