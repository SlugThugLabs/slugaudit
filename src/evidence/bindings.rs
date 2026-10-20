//! Variable-binding and field-declaration walker.
//!
//! The language pack's `process()` extracts structure, imports, exports,
//! comments, docstrings, symbols, and diagnostics — but not individual
//! variable bindings or struct/class fields. Those are exactly the items an
//! AI needs to spot near-duplicate names (`file_path` vs `filepath` vs
//! `filePath`) that probably refer to the same thing. This module walks the
//! tree with generic node-type patterns that are consistent across
//! tree-sitter grammars, so no per-language query code is needed.

use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, LineIndex};
use serde_json::json;
use tree_sitter_language_pack::SymbolKind;

use super::TextRangeError;
use super::{node_span, node_text};

/// Node-type patterns that indicate an individual binding. These names are
/// shared across tree-sitter language grammars — `let_declaration` in Rust,
/// JS, Go, etc.; `field_declaration` in Rust, C#, Java; `parameter`
/// universally. Matching on the name rather than a per-language query is
/// what keeps this generic.
fn is_variable_binding(kind: &str) -> bool {
    matches!(
        kind,
        "let_declaration"
            | "const_declaration"
            | "var_declaration"
            | "val_declaration"
            | "binding"
            | "parameter"
            | "assignment"
    )
}

/// Exact node kinds for a *field* declaration, verified against a fixed
/// grammar matrix (C7): rust/go/c/cpp/csharp/ocaml/java use
/// `field_declaration`; swift/kotlin use `property_declaration`;
/// javascript uses `field_definition`; typescript uses
/// `public_field_definition` and (for interface members) `property_signature`.
///
/// Exact match only — never `contains`: a substring test on
/// `field_declaration` also matches the *container* node
/// `field_declaration_list` (and could match unrelated kinds in an
/// unforeseen grammar), which would emit a duplicate Field symbol for the
/// same member. `field_identifier` (the name child) is deliberately not a
/// declaration and is not listed.
fn is_field_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "field_declaration"
            | "property_declaration"
            | "field_definition"
            | "public_field_definition"
            | "property_signature"
    )
}

/// Pre-order walk over an explicit heap stack (never recursion — see
/// [`crate::evidence::push_children`]) emitting one symbol per named binding
/// or field. Children are visited regardless of a match — a function body
/// contains let bindings, a struct body contains field declarations, etc.
pub(super) fn walk_bindings(
    root: &tree_sitter_language_pack::Node,
    source: &str,
    lines: &LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) -> bool {
    let mut capped = false;
    let mut pending = vec![(root.clone(), 0)];
    while let Some((node, depth)) = pending.pop() {
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            capped = true;
            continue;
        }
        match identifier_name(&node, source) {
            Ok(Some(name)) => push_binding(&node, name, lines, items, counter),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "binding name skipped; extraction is partial");
                capped = true;
            }
        }
        super::push_children(&mut pending, &node, |child| (child, depth + 1));
    }
    capped
}

/// Emits the symbol row for `node` when its kind is a field or a variable
/// binding; any other named node is not a binding and emits nothing.
fn push_binding(
    node: &tree_sitter_language_pack::Node,
    name: String,
    lines: &LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
) {
    let kind = node.kind();
    let (sym_kind, origin_kind) = if is_field_declaration(&kind) {
        (SymbolKind::Other("Field".to_owned()), "field")
    } else if is_variable_binding(&kind) {
        if kind.starts_with("const") {
            (SymbolKind::Constant, "variable")
        } else {
            (SymbolKind::Variable, "variable")
        }
    } else {
        return;
    };
    let key = format!("{}:{counter}", origin_kind);
    *counter += 1;
    items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::PackSymbol,
        span: node_span(node, lines, "binding"),
        payload: json!({
            "name": name,
            "kind": format!("{sym_kind:?}"),
            "type_annotation": null,
            "doc": null,
        }),
    });
}

/// Returns the text of the first identifier-flavored named child of `node`,
/// if any. This is the binding name for every node type this extractor
/// targets. Different grammars spell the name node differently — Rust
/// struct fields use `field_identifier`, type aliases use `type_identifier`,
/// and most everything else uses plain `identifier` — so we match any of
/// them rather than hard-coding one. The pack exposes no `utf8_text`
/// helper, so the text is sliced directly from `source` using the child's
/// byte range.
///
/// When no direct child is an identifier, falls back to a bounded
/// recursive search of the declaration node's subtree (C7 grammar-matrix
/// finding): swift/kotlin nest the name under `pattern` /
/// `variable_declaration` wrappers, so `property_declaration`'s direct
/// children are not identifiers. The search returns the *first*
/// identifier-kind descendant — for the probed grammars (rust, go, c,
/// cpp, csharp, java, ocaml, swift, kotlin, js/ts) the member name
/// precedes the type, so this yields the name, not the type.
fn identifier_name(
    node: &tree_sitter_language_pack::Node,
    source: &str,
) -> Result<Option<String>, TextRangeError> {
    for child in named_children(node) {
        if is_identifier_kind(&child.kind()) {
            return Ok(Some(
                node_text(source, child.start_byte(), child.end_byte())?.to_owned(),
            ));
        }
    }
    first_identifier_descendant(node, source, 0)
}

/// Depth-first search for the first identifier-kind descendant. `depth`
/// bounds the descent so a pathological grammar can't turn a single
/// declaration into an unbounded traversal — two levels of wrapper
/// (e.g. `property_declaration → pattern → simple_identifier`) is the
/// deepest any probed grammar nests a member name.
fn first_identifier_descendant(
    node: &tree_sitter_language_pack::Node,
    source: &str,
    depth: u8,
) -> Result<Option<String>, TextRangeError> {
    if depth >= 3 {
        return Ok(None);
    }
    for child in named_children(node) {
        if is_identifier_kind(&child.kind()) {
            return Ok(Some(
                node_text(source, child.start_byte(), child.end_byte())?.to_owned(),
            ));
        }
    }
    for child in named_children(node) {
        if let Some(name) = first_identifier_descendant(&child, source, depth + 1)? {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Node kinds that name a *binding or member*. Deliberately excludes
/// `type_identifier`/`type_name`: in Java (and other grammars) a
/// `field_declaration` lists the type before the name, so treating the
/// type node as a name candidate would emit the type as the member name
/// (a C7 grammar-matrix finding). Type-name nodes are never the *name* of
/// a binding this walker targets.
fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "field_name"
            | "shorthand_property_identifier"
            | "property_identifier"
            | "simple_identifier"
    )
}

/// Iterator helper over a node's named children. The pack only exposes
/// `named_child(index)` + `named_child_count()`, so we build the iteration
/// ourselves rather than depending on an iterator method that doesn't
/// exist.
fn named_children(
    node: &tree_sitter_language_pack::Node,
) -> impl Iterator<Item = tree_sitter_language_pack::Node> {
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}
