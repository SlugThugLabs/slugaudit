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
//! variable-binding walker in `bindings` — and emits `Import` evidence for
//! any node whose kind denotes a whole import statement. It runs only when
//! the pack produced zero imports for a file, so pack-covered languages
//! never double-report; it reads the tree the binding walker already parsed.
//!
//! Node kinds are matched by *name* rather than per-language query; the
//! resulting `source` text is the raw statement sliced from the original
//! source, exactly like the pack's own `ImportInfo.source`.

use super::{Extraction, WalkInput};
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin};
use serde_json::json;
use tree_sitter_language_pack::Node;

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
/// Exact match only. The bare `"import"` kind is kept because
/// Haskell's grammar names its import statement `import` (verified by the
/// `haskell_import_is_captured` matrix test); the matrix's no-imports test
/// guarantees a non-import
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

/// Walk emitting one `Import` evidence item per import-shaped
/// node, slicing the raw statement text from the source via byte range.
/// Child nodes of a matched statement are still visited (a `using` block
/// may contain nested directives), but only whole-statement kinds emit.
///
/// A matched node whose **parent** is also an import-shaped node is
/// skipped: some grammars nest the real import inside a wrapper that is
/// itself import-shaped (dart: `import_or_export` → `library_import` →
/// `import_specification`; haskell: `imports` → `import`). Emitting at
/// both levels would double-report the same statement. The outermost
/// matched node owns the emit;
/// nested matches are containers, not statements.
///
/// `pub(super)` so `normalize.rs::extract_extra_walkers` can run this
/// walker on the same already-parsed tree it uses for the
/// variable-binding walker — saving another parse of the same source.
///
/// The walk is a loop over an explicit heap stack, never recursion (see
/// [`super::push_children`]): each entry carries whether it is *suppressed*
/// — an import-shaped node directly under another import-shaped node, which
/// is walked past without emitting.
///
/// Pushes its items onto `out.items`. Sets `out.truncated` when the walk was
/// cut short by the depth bound or an unsliceable statement, and
/// `out.timed_out` (stopping the walk) when the deadline ran out; a position
/// past `u32::MAX` is an error that fails the file's extraction.
pub(super) fn walk_imports(
    root: &Node,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<(), crate::model::PositionOverflow> {
    let mut pending = vec![(root.clone(), 0, false)];
    while let Some((node, depth, suppressed)) = pending.pop() {
        if super::deadline_spent(input.deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        let is_import = is_import_statement_kind(&node.kind());
        if is_import && !suppressed {
            // Trim: some grammars (e.g. c/cpp `preproc_include`) include the
            // trailing newline in the statement node's byte range.
            match super::node_text(input.source, node.start_byte(), node.end_byte()) {
                Ok(text) => push_import(text.trim(), &node, input.lines, &mut out.items, counter)?,
                Err(error) => {
                    tracing::warn!(%error, "import statement skipped; extraction is partial");
                    out.truncated = true;
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
    Ok(())
}

fn push_import(
    text: &str,
    node: &Node,
    lines: &crate::model::LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) -> Result<(), crate::model::PositionOverflow> {
    let key = format!("generic_import:{counter}");
    *counter += 1;
    items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::GenericWalker,
        span: super::node_span(node, lines, "generic import")?,
        payload: json!({
            "source": text,
            "items": [],
            "alias": null,
            "is_wildcard": text.contains('*'),
        }),
    });
    Ok(())
}

#[cfg(test)]
#[path = "generic_imports_tests.rs"]
mod tests;
