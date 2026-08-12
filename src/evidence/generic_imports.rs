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

use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, Position, Span, SpanAvailability};
use serde_json::json;
use tree_sitter_language_pack::{Node, get_parser};

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
/// parser or the source doesn't parse — callers treat that as \"no imports
/// found\", not an error, matching `extract_bindings`'s contract.
pub(super) fn extract_generic_imports(language: &str, source: &str) -> Vec<EvidenceItem> {
    let Ok(mut parser) = get_parser(language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(source) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut counter = 0usize;
    walk_imports(&tree.root_node(), source, &mut items, &mut counter);
    items
}

/// Recursive walk emitting one `Import` evidence item per import-shaped
/// node, slicing the raw statement text from `source` via byte range.
/// Child nodes of a matched statement are still visited (a `using` block
/// may contain nested directives), but only whole-statement kinds emit.
fn walk_imports(node: &Node, source: &str, items: &mut Vec<EvidenceItem>, counter: &mut usize) {
    let kind = node.kind();
    if is_import_statement_kind(&kind) {
        // Trim: some grammars (e.g. c/cpp `preproc_include`) include the
        // trailing newline in the statement node's byte range.
        let text = source[node.start_byte()..node.end_byte()].trim();
        let key = format!("generic_import:{counter}");
        *counter += 1;
        items.push(EvidenceItem {
            key,
            kind: EvidenceKind::Import,
            origin: EvidenceOrigin::GenericWalker,
            span: node_span(node, source),
            payload: json!({
                "source": text,
                "items": [],
                "alias": null,
                "is_wildcard": text.contains('*'),
            }),
        });
    }
    for child in named_children(node) {
        walk_imports(&child, source, items, counter);
    }
}

/// Iterator helper over a node's named children (the pack exposes only
/// `named_child(index)` + `named_child_count()`).
fn named_children(node: &Node) -> impl Iterator<Item = Node> {
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}

fn node_span(node: &Node, source: &str) -> SpanAvailability {
    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    let start = Position {
        line: crate::model::saturating_u32(node.start_position().row),
        column: crate::model::char_column(source, start_byte),
    };
    let end = Position {
        line: crate::model::saturating_u32(node.end_position().row),
        column: crate::model::char_column(source, end_byte),
    };
    match Span::new(start_byte as u64, end_byte as u64, start, end) {
        Ok(span) => SpanAvailability::Present(span),
        Err(_) => SpanAvailability::NormalizerUnavailable {
            reason: "generic import span failed local range validation".into(),
        },
    }
}

#[cfg(test)]
#[path = "generic_imports_tests.rs"]
mod tests;
