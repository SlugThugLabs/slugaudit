// slugaudit-line-exception: approved-by=split-judge; reason=Rust GenericWalker binding, parameter, closure-parameter, field, enum-variant, static, and destructuring-pattern extraction
//! Rust-specific `GenericWalker` `Symbol` emitter for destructuring patterns,
//! `let` / `if let` / `for` / `match` arm bindings, function and closure
//! parameters, struct and union fields, enum variants, and `static_item`
//! declarations.

use super::bindings::{BindingFields, push_binding_span};
use super::rust_items::declaration_span;
use super::rust_visibility::rust_visibility;
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
        "let_condition" | "for_expression" | "match_arm" => {
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
            Ok(push_binding_span(
                declaration_span(node, input.lines, "binding")?,
                &fields,
                out,
                counter,
            ))
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
            Ok(push_binding_span(
                declaration_span(node, input.lines, "binding")?,
                &fields,
                out,
                counter,
            ))
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
            "line_comment"
            | "block_comment"
            | "line_outer_doc_comment"
            | "block_outer_doc_comment"
            | "attribute_item"
            | "attributes" => {}
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
    let span = declaration_span(span_node, input.lines, "binding")?;
    for name in &names {
        let fields = BindingFields {
            name,
            sym_kind: "Variable",
            prefix: "variable",
            visibility: None,
            type_annotation,
            doc: None,
        };
        if !push_binding_span(span.clone(), &fields, out, counter) {
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatternIdent {
    BindingOrPath,
    ShorthandField,
}

/// Collects every bound variable identifier inside a Rust `pattern` subtree in
/// source order, excluding struct/enum constructor paths, explicit field labels
/// before `:` in `field_pattern`, `match_pattern` `if` guards, `self`, range
/// bounds, wildcards, and unit-variant / const patterns.
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
            "identifier" => {
                push_unique_name(node, PatternIdent::BindingOrPath, source, names, out);
            }
            "shorthand_field_identifier" => {
                push_unique_name(node, PatternIdent::ShorthandField, source, names, out);
            }
            "field_pattern" => {
                if let Some(subpattern) = node.child_by_field_name("pattern") {
                    pending.push((subpattern, next_depth));
                } else if let Some(shorthand) = node.child_by_field_name("name") {
                    push_unique_name(shorthand, PatternIdent::ShorthandField, source, names, out);
                }
            }
            "tuple_struct_pattern" => {
                push_children_excluding_field(node, "type", next_depth, &mut pending);
            }
            "match_pattern" => {
                push_children_excluding_field(node, "condition", next_depth, &mut pending);
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
            | "generic_pattern"
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

fn push_children_excluding_field<'tree>(
    node: Node<'tree>,
    excluded_field: &str,
    next_depth: u32,
    pending: &mut Vec<(Node<'tree>, u32)>,
) {
    let excluded_range = node
        .child_by_field_name(excluded_field)
        .map(|excluded| (excluded.start_byte(), excluded.end_byte()));
    let first = pending.len();
    for child in named_children(node) {
        if Some((child.start_byte(), child.end_byte())) != excluded_range {
            pending.push((child, next_depth));
        }
    }
    reverse_from(pending, first);
}

fn is_const_or_variant_name(text: &str) -> bool {
    let bare = match text.strip_prefix("r#") {
        Some(rest) => rest,
        None => text,
    };
    bare.chars().next().is_some_and(char::is_uppercase)
}

fn push_unique_name(
    node: Node<'_>,
    ident_kind: PatternIdent,
    source: &str,
    names: &mut Vec<String>,
    out: &mut Extraction,
) {
    match node_text(source, node.start_byte(), node.end_byte()) {
        Ok(text) => {
            let skip = text.is_empty()
                || text == "_"
                || (ident_kind == PatternIdent::BindingOrPath && is_const_or_variant_name(text))
                || names.iter().any(|existing| existing == text);
            if !skip {
                names.push(text.to_owned());
            }
        }
        Err(error) => {
            tracing::warn!(%error, "Rust pattern identifier skipped; extraction is partial");
            out.truncated = true;
        }
    }
}
