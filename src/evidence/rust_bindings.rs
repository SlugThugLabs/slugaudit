// slugaudit-line-exception: approved-by=split-judge; reason=Rust GenericWalker binding, parameter, closure-parameter, field, enum-variant, static, and destructuring-pattern extraction
//! Rust-specific `GenericWalker` `Symbol` emitter for destructuring patterns,
//! `let` / `if let` / `for` bindings, function and closure parameters, struct
//! and union fields, enum variants, and `static_item` declarations.

use super::bindings::{BindingFields, push_binding};
use super::rust_doc::rust_visibility;
use super::{Extraction, WalkInput, named_children, node_text, reverse_from};
use crate::model::PositionOverflow;
use tree_sitter::Node;

/// Emits `GenericWalker` `Symbol` rows when `node` is a Rust local binding,
/// parameter, closure parameter list, field, enum variant, or `static_item`.
/// Returns `Ok(false)` only if the per-file binding key counter overflows.
pub(super) fn emit_rust_binding_node(
    node: Node<'_>,
    in_closure_params: bool,
    outer_doc: Option<&str>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<bool, PositionOverflow> {
    match node.kind() {
        "parameter" if in_closure_params => Ok(true),
        "let_declaration" | "parameter" => {
            let Some(pattern) = node.child_by_field_name("pattern") else {
                return Ok(true);
            };
            let ty = slice_field(node, "type", input.source, out);
            emit_pattern_bindings(node, pattern, ty.as_deref(), input, out, counter)
        }
        "let_condition" | "for_expression" => {
            let Some(pattern) = node.child_by_field_name("pattern") else {
                return Ok(true);
            };
            emit_pattern_bindings(node, pattern, None, input, out, counter)
        }
        "closure_parameters" => emit_closure_parameters(node, input, out, counter),
        kind @ ("field_declaration" | "static_item") => {
            let Some(name) = slice_field(node, "name", input.source, out) else {
                return Ok(true);
            };
            let ty = slice_field(node, "type", input.source, out);
            let vis = rust_visibility(node, input.source, out);
            let (sym_kind, prefix) = if kind == "field_declaration" {
                ("Other(\"Field\")", "field")
            } else if named_children(node).any(|child| child.kind() == "mutable_specifier") {
                ("Variable", "variable")
            } else {
                ("Constant", "variable")
            };
            let fields = BindingFields {
                name: &name,
                sym_kind,
                prefix,
                visibility: vis.as_deref(),
                type_annotation: ty.as_deref(),
                doc: outer_doc,
            };
            push_binding(node, &fields, input.lines, out, counter)
        }
        "enum_variant" => {
            let Some(name) = slice_field(node, "name", input.source, out) else {
                return Ok(true);
            };
            let vis = rust_visibility(node, input.source, out);
            let fields = BindingFields {
                name: &name,
                sym_kind: "Constant",
                prefix: "variable",
                visibility: vis.as_deref(),
                type_annotation: None,
                doc: outer_doc,
            };
            push_binding(node, &fields, input.lines, out, counter)
        }
        _ => Ok(true),
    }
}

fn emit_closure_parameters(
    params_node: Node<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<bool, PositionOverflow> {
    for child in named_children(params_node) {
        match child.kind() {
            "line_comment" | "block_comment" | "attribute_item" => {}
            "parameter" => {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    let ty = slice_field(child, "type", input.source, out);
                    if !emit_pattern_bindings(child, pattern, ty.as_deref(), input, out, counter)? {
                        return Ok(false);
                    }
                }
            }
            _ => {
                if !emit_pattern_bindings(child, child, None, input, out, counter)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn emit_pattern_bindings(
    span_node: Node<'_>,
    pattern: Node<'_>,
    type_annotation: Option<&str>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<bool, PositionOverflow> {
    let mut names = Vec::new();
    collect_rust_pattern_names(pattern, input.source, &mut names, out);
    for name in &names {
        let fields = BindingFields {
            name,
            sym_kind: "Variable",
            prefix: "variable",
            visibility: None,
            type_annotation,
            doc: None,
        };
        if !push_binding(span_node, &fields, input.lines, out, counter)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn slice_field(
    node: Node<'_>,
    field_name: &str,
    source: &str,
    out: &mut Extraction,
) -> Option<String> {
    let child = node.child_by_field_name(field_name)?;
    match node_text(source, child.start_byte(), child.end_byte()) {
        Ok(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Err(error) => {
            tracing::warn!(%error, field = field_name, "Rust CST field slice skipped; extraction is partial");
            out.truncated = true;
            None
        }
    }
}

/// Collects every bound variable identifier inside a Rust `pattern` subtree in
/// source order, excluding struct/enum constructor paths, explicit field labels
/// before `:` in `field_pattern`, `self`, range bounds, and const/macro patterns.
fn collect_rust_pattern_names(
    pattern: Node<'_>,
    source: &str,
    names: &mut Vec<String>,
    out: &mut Extraction,
) {
    let mut pending: Vec<(Node<'_>, u32)> = vec![(pattern, 0)];
    while let Some((node, depth)) = pending.pop() {
        if depth >= super::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        let Some(next_depth) = depth.checked_add(1) else {
            out.truncated = true;
            continue;
        };
        match node.kind() {
            "identifier" | "shorthand_field_identifier" => {
                push_unique_name(node, source, names, out);
            }
            "field_pattern" => {
                if let Some(subpattern) = node.child_by_field_name("pattern") {
                    pending.push((subpattern, next_depth));
                } else if let Some(shorthand) = node.child_by_field_name("name") {
                    push_unique_name(shorthand, source, names, out);
                }
            }
            "tuple_struct_pattern" => {
                let ctor_range = node
                    .child_by_field_name("type")
                    .map(|ctor| (ctor.start_byte(), ctor.end_byte()));
                let first = pending.len();
                for child in named_children(node) {
                    if Some((child.start_byte(), child.end_byte())) != ctor_range {
                        pending.push((child, next_depth));
                    }
                }
                reverse_from(&mut pending, first);
            }
            "struct_pattern" => {
                let first = pending.len();
                for child in named_children(node) {
                    if child.kind() == "field_pattern" {
                        pending.push((child, next_depth));
                    }
                }
                reverse_from(&mut pending, first);
            }
            "scoped_identifier"
            | "scoped_type_identifier"
            | "type_identifier"
            | "range_pattern"
            | "remaining_field_pattern"
            | "macro_invocation"
            | "const_block"
            | "self" => {}
            _ => {
                super::push_children(&mut pending, node, |child| (child, next_depth));
            }
        }
    }
}

fn push_unique_name(node: Node<'_>, source: &str, names: &mut Vec<String>, out: &mut Extraction) {
    match node_text(source, node.start_byte(), node.end_byte()) {
        Ok(text) => {
            if !text.is_empty() && !names.iter().any(|existing| existing == text) {
                names.push(text.to_owned());
            }
        }
        Err(error) => {
            tracing::warn!(%error, "Rust pattern identifier skipped; extraction is partial");
            out.truncated = true;
        }
    }
}
