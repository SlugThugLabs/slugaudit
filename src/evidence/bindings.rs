//! Non-Rust variable-binding and field-declaration CST walker.
//!
//! Rust files use the single-parse CST extractor in [`super::rust_items`] and
//! [`super::rust_bindings`]; other languages use the generic node-kind matcher
//! here over the shared secondary `tree_sitter::Tree`.

use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, EvidencePayload, LineIndex, PositionOverflow,
    SymbolPayload,
};

use super::{Extraction, TextRangeError, WalkInput, named_children, node_span, node_text};

/// Payload fields for a single [`EvidenceOrigin::GenericWalker`] `Symbol` row.
pub(super) struct BindingFields<'a> {
    pub(super) name: &'a str,
    pub(super) sym_kind: &'static str,
    pub(super) prefix: &'static str,
    pub(super) visibility: Option<&'a str>,
    pub(super) type_annotation: Option<&'a str>,
    pub(super) doc: Option<&'a str>,
}

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

/// Pre-order walk over an explicit heap stack pushing onto `out.items` one
/// symbol per named binding or field for non-Rust languages.
pub(super) fn walk_bindings(
    root: tree_sitter::Node<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<(), PositionOverflow> {
    let mut pending = vec![(root, 0u32)];
    while let Some((node, depth)) = pending.pop() {
        if super::deadline_spent(input.deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        if let Some((sym_kind, prefix)) = binding_kind(node.kind()) {
            match identifier_name(node, input.source) {
                Ok(Some(name)) => {
                    let fields = BindingFields {
                        name: &name,
                        sym_kind,
                        prefix,
                        visibility: None,
                        type_annotation: None,
                        doc: None,
                    };
                    if !push_binding(node, &fields, input.lines, out, counter)? {
                        break;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "binding name skipped; extraction is partial");
                    out.truncated = true;
                }
            }
        }
        let Some(child_depth) = depth.checked_add(1) else {
            out.truncated = true;
            continue;
        };
        super::push_children(&mut pending, node, |child| (child, child_depth));
    }
    Ok(())
}

/// Appends one `Symbol` row for `node` and increments `counter`, returning
/// `Ok(false)` when `counter` overflows `usize::MAX`.
pub(super) fn push_binding(
    node: tree_sitter::Node<'_>,
    fields: &BindingFields<'_>,
    lines: &LineIndex<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<bool, PositionOverflow> {
    let Some(next) = counter.checked_add(1) else {
        tracing::warn!("binding key counter exhausted; extraction is partial");
        out.truncated = true;
        return Ok(false);
    };
    let item = binding_item(node, fields, *counter, lines)?;
    *counter = next;
    out.items.push(item);
    Ok(true)
}

fn binding_kind(kind: &str) -> Option<(&'static str, &'static str)> {
    if is_field_declaration(kind) {
        Some(("Other(\"Field\")", "field"))
    } else if is_variable_binding(kind) {
        if kind.starts_with("const") {
            Some(("Constant", "variable"))
        } else {
            Some(("Variable", "variable"))
        }
    } else {
        None
    }
}

fn binding_item(
    node: tree_sitter::Node<'_>,
    fields: &BindingFields<'_>,
    index: usize,
    lines: &LineIndex<'_>,
) -> Result<EvidenceItem, PositionOverflow> {
    Ok(EvidenceItem {
        key: format!("{}:{index}", fields.prefix),
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::GenericWalker,
        span: node_span(node, lines, "binding")?,
        payload: EvidencePayload::Symbol(SymbolPayload {
            name: fields.name.to_owned(),
            kind: fields.sym_kind.to_owned(),
            visibility: fields.visibility.map(str::to_owned),
            type_annotation: fields.type_annotation.map(str::to_owned),
            doc: fields.doc.map(str::to_owned),
        }),
    })
}

fn identifier_name(
    node: tree_sitter::Node<'_>,
    source: &str,
) -> Result<Option<String>, TextRangeError> {
    for child in named_children(node) {
        if is_identifier_kind(child.kind()) {
            return Ok(Some(
                node_text(source, child.start_byte(), child.end_byte())?.to_owned(),
            ));
        }
    }
    first_identifier_descendant(node, source, 0)
}

fn first_identifier_descendant(
    node: tree_sitter::Node<'_>,
    source: &str,
    depth: u8,
) -> Result<Option<String>, TextRangeError> {
    if depth >= 3 {
        return Ok(None);
    }
    for child in named_children(node) {
        if is_identifier_kind(child.kind()) {
            return Ok(Some(
                node_text(source, child.start_byte(), child.end_byte())?.to_owned(),
            ));
        }
    }
    let Some(next_depth) = depth.checked_add(1) else {
        return Ok(None);
    };
    for child in named_children(node) {
        if let Some(name) = first_identifier_descendant(child, source, next_depth)? {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

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
