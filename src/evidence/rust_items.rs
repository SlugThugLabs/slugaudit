// slugaudit-line-exception: approved-by=split-judge; reason=single-pass iterative Rust CST extractor for Structure, Symbol, Import, Comment, and Diagnostic evidence
//! Single-pass iterative `tree-sitter-rust` CST extractor for `Structure`,
//! `Symbol`, `Import`, `Comment`, and `Diagnostic` evidence.

use super::rust_bindings::{emit_rust_binding_node, slice_field};
use super::rust_comment::classify_rust_comment;
use super::rust_doc::{
    NodeMeta, SiblingTracker, content_end_row, declaration_start_node, is_rust_comment_kind,
};
use super::rust_mod_paths::{InlineModStack, ModItemDisposition};
use super::rust_visibility::rust_visibility;
use super::{Extraction, WalkInput, node_span, node_text, reverse_from};
use crate::model::{
    CommentPayload, DiagnosticPayload, EvidenceItem, EvidenceKind, EvidenceOrigin, EvidencePayload,
    ImportPayload, PositionOverflow, RustModPathPayload, StructurePayload, SymbolPayload,
};
use tree_sitter::{Node, Tree};

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnclosingScope {
    ModuleOrBlock,
    ImplOrTrait,
    ForeignMod,
    ClosureParams,
}

enum Step<'tree> {
    Visit {
        node: Node<'tree>,
        depth: u32,
        scope: EnclosingScope,
        meta: NodeMeta<'tree>,
    },
    PopInlineMod,
}

#[derive(Default)]
struct Counters {
    structure: usize,
    import: usize,
    comment: usize,
    symbol: usize,
    diagnostic: usize,
    binding: usize,
}

/// Walks a parsed Rust `tree` once using an explicit heap stack bounded by
/// [`super::MAX_WALK_DEPTH`] and `input.deadline`, emitting typed `Structure`,
/// `Symbol`, `Import`, `Comment`, and `Diagnostic` evidence items.
pub(super) fn extract_rust_cst(
    tree: &Tree,
    input: &WalkInput<'_>,
) -> Result<Extraction, PositionOverflow> {
    let mut out = Extraction::default();
    let mut counters = Counters::default();
    let mut inline_mods = InlineModStack::default();
    let mut pending = vec![Step::Visit {
        node: tree.root_node(),
        depth: 0,
        scope: EnclosingScope::ModuleOrBlock,
        meta: NodeMeta::default(),
    }];

    while let Some(step) = pending.pop() {
        let Step::Visit {
            node,
            depth,
            scope,
            meta,
        } = step
        else {
            inline_mods.pop();
            continue;
        };
        if super::deadline_spent(input.deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= super::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }

        if node.is_error() || node.is_missing() {
            emit_diagnostic(node, input, &mut out, &mut counters)?;
        }

        let pop_inline_after = emit_item_evidence(
            node,
            scope,
            &meta,
            &mut inline_mods,
            input,
            &mut out,
            &mut counters,
        )?;
        if !emit_rust_binding_node(
            node,
            scope == EnclosingScope::ClosureParams,
            meta.outer_doc.as_deref(),
            input,
            &mut out,
            &mut counters.binding,
        )? {
            break;
        }

        let Some(child_depth) = depth.checked_add(1) else {
            out.truncated = true;
            continue;
        };
        if pop_inline_after {
            pending.push(Step::PopInlineMod);
        }
        push_node_children(
            &mut pending,
            node,
            child_depth,
            child_scope_for(node, scope),
            input,
            &mut out,
            &mut counters,
        )?;
    }
    Ok(out)
}

fn child_scope_for(node: Node<'_>, current: EnclosingScope) -> EnclosingScope {
    match node.kind() {
        "impl_item" | "trait_item" => EnclosingScope::ImplOrTrait,
        "foreign_mod_item" => EnclosingScope::ForeignMod,
        "declaration_list"
            if matches!(
                current,
                EnclosingScope::ImplOrTrait | EnclosingScope::ForeignMod
            ) =>
        {
            current
        }
        "closure_parameters" => EnclosingScope::ClosureParams,
        _ => EnclosingScope::ModuleOrBlock,
    }
}

fn is_attribute_container(kind: &str) -> bool {
    matches!(
        kind,
        "source_file"
            | "declaration_list"
            | "block"
            | "field_declaration_list"
            | "ordered_field_declaration_list"
            | "enum_variant_list"
            | "struct_pattern"
            | "type_parameters"
            | "array_expression"
            | "tuple_expression"
            | "ERROR"
    )
}

pub(super) fn declaration_span(
    node: Node<'_>,
    lines: &crate::model::LineIndex<'_>,
    context: &'static str,
) -> Result<crate::model::SpanAvailability, PositionOverflow> {
    let start = declaration_start_node(node);
    super::range_span(
        (start.start_byte(), node.end_byte()),
        (start.start_position().row, node.end_position().row),
        lines,
        context,
    )
}

fn push_node_children<'tree>(
    pending: &mut Vec<Step<'tree>>,
    node: Node<'tree>,
    child_depth: u32,
    child_scope: EnclosingScope,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let first = pending.len();
    let mut tracker = SiblingTracker::default();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() {
                if is_rust_comment_kind(child.kind()) {
                    let next_sibling = child.next_named_sibling().or_else(|| {
                        (node.kind() == "attributes")
                            .then(|| {
                                let parent = node.parent()?;
                                if is_attribute_container(parent.kind()) {
                                    node.next_named_sibling()
                                } else {
                                    Some(parent)
                                }
                            })
                            .flatten()
                    });
                    emit_comment(child, next_sibling, input, out, counters)?;
                    let _step = tracker.step(child, input.source, out);
                } else if child.kind() == "attributes" {
                    if is_attribute_container(node.kind()) {
                        tracker.push_attributes_node(child, input.source, out);
                    }
                    pending.push(Step::Visit {
                        node: child,
                        depth: child_depth,
                        scope: child_scope,
                        meta: NodeMeta::default(),
                    });
                } else if let Some(meta) = tracker.step(child, input.source, out) {
                    pending.push(Step::Visit {
                        node: child,
                        depth: child_depth,
                        scope: child_scope,
                        meta,
                    });
                }
            } else if child.is_missing() {
                emit_diagnostic(child, input, out, counters)?;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    reverse_from(pending, first);
    Ok(())
}

fn emit_item_evidence<'tree>(
    node: Node<'tree>,
    scope: EnclosingScope,
    meta: &NodeMeta<'tree>,
    inline_mods: &mut InlineModStack,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<bool, PositionOverflow> {
    match node.kind() {
        "function_item" => {
            let kind = if scope == EnclosingScope::ImplOrTrait {
                "Method"
            } else {
                "Function"
            };
            emit_callable_item(node, kind, meta, input, out, counters)?;
            Ok(false)
        }
        "function_signature_item" => {
            let kind = if scope == EnclosingScope::ForeignMod {
                "Function"
            } else {
                "Method"
            };
            emit_callable_item(node, kind, meta, input, out, counters)?;
            Ok(false)
        }
        "struct_item" => emit_nominal(node, "Struct", "Struct", meta, input, out, counters),
        "union_item" => emit_nominal(node, "Union", "Type", meta, input, out, counters),
        "enum_item" => emit_nominal(node, "Enum", "Enum", meta, input, out, counters),
        "trait_item" => emit_nominal(node, "Trait", "Trait", meta, input, out, counters),
        "impl_item" => {
            let name = impl_name(node, input.source, out);
            let vis = rust_visibility(node, input.source, out);
            emit_structure(node, ("Impl", name, vis, None), meta, input, out, counters)?;
            Ok(false)
        }
        "mod_item" => emit_mod_item(node, meta, inline_mods, input, out, counters),
        "macro_definition" | "macro_definition_v2" => {
            emit_callable_item(node, "Macro", meta, input, out, counters)
        }
        "type_item" | "associated_type" => emit_type_item(node, meta, input, out, counters),
        "const_item" => emit_const_item(node, meta, input, out, counters),
        "use_declaration" | "extern_crate_declaration" => {
            emit_import(node, inline_mods, input, out, counters)?;
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn emit_nominal(
    node: Node<'_>,
    struct_kind: &'static str,
    sym_kind: &'static str,
    meta: &NodeMeta<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<bool, PositionOverflow> {
    emit_nominal_item(node, struct_kind, sym_kind, meta, input, out, counters)?;
    Ok(false)
}

fn emit_callable_item(
    node: Node<'_>,
    struct_kind: &'static str,
    meta: &NodeMeta<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<bool, PositionOverflow> {
    let sym_kind = if struct_kind == "Macro" {
        "Macro"
    } else {
        "Function"
    };
    let name = slice_field(node, "name", input.source, out);
    let vis = rust_visibility(node, input.source, out);
    emit_structure(
        node,
        (struct_kind, name.clone(), vis.clone(), None),
        meta,
        input,
        out,
        counters,
    )?;
    if let Some(sym_name) = name {
        emit_pack_symbol(
            node,
            (sym_name, sym_kind, vis, None, meta.outer_doc.clone()),
            input,
            out,
            counters,
        )?;
    }
    Ok(false)
}

fn emit_mod_item(
    node: Node<'_>,
    meta: &NodeMeta<'_>,
    inline_mods: &mut InlineModStack,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<bool, PositionOverflow> {
    let name = slice_field(node, "name", input.source, out);
    let vis = rust_visibility(node, input.source, out);
    let (pop_after, mod_path) =
        match inline_mods.enter_mod_item(node, name.as_deref(), &meta.outer_attrs, input.source) {
            ModItemDisposition::Inline { pushed } => (pushed, None),
            ModItemDisposition::External(payload) => (false, payload),
        };
    emit_structure(
        node,
        ("Module", name.clone(), vis.clone(), mod_path),
        meta,
        input,
        out,
        counters,
    )?;
    if let Some(sym_name) = name {
        emit_pack_symbol(
            node,
            (sym_name, "Module", vis, None, meta.outer_doc.clone()),
            input,
            out,
            counters,
        )?;
    }
    Ok(pop_after)
}

fn emit_type_item(
    node: Node<'_>,
    meta: &NodeMeta<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<bool, PositionOverflow> {
    if let Some(name) = slice_field(node, "name", input.source, out) {
        let vis = rust_visibility(node, input.source, out);
        let ty = slice_field(node, "type", input.source, out).or_else(|| {
            slice_field(node, "bounds", input.source, out)
                .map(|b| b.trim_start_matches(':').trim().to_owned())
                .filter(|b| !b.is_empty())
        });
        emit_pack_symbol(
            node,
            (name, "Type", vis, ty, meta.outer_doc.clone()),
            input,
            out,
            counters,
        )?;
    }
    Ok(false)
}

fn emit_const_item(
    node: Node<'_>,
    meta: &NodeMeta<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<bool, PositionOverflow> {
    if let Some(name) = slice_field(node, "name", input.source, out) {
        let vis = rust_visibility(node, input.source, out);
        let ty = slice_field(node, "type", input.source, out);
        emit_pack_symbol(
            node,
            (name, "Constant", vis, ty, meta.outer_doc.clone()),
            input,
            out,
            counters,
        )?;
    }
    Ok(false)
}

fn emit_nominal_item(
    node: Node<'_>,
    struct_kind: &'static str,
    sym_kind: &'static str,
    meta: &NodeMeta<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let name = slice_field(node, "name", input.source, out);
    let vis = rust_visibility(node, input.source, out);
    emit_structure(
        node,
        (struct_kind, name.clone(), vis.clone(), None),
        meta,
        input,
        out,
        counters,
    )?;
    if let Some(sym_name) = name {
        emit_pack_symbol(
            node,
            (sym_name, sym_kind, vis, None, meta.outer_doc.clone()),
            input,
            out,
            counters,
        )?;
    }
    Ok(())
}

fn impl_name(node: Node<'_>, source: &str, out: &mut Extraction) -> Option<String> {
    let ty = slice_field(node, "type", source, out)?;
    match slice_field(node, "trait", source, out) {
        Some(tr) => Some(format!("{tr} for {ty}")),
        None => Some(ty),
    }
}

fn item_signature(node: Node<'_>, source: &str, out: &mut Extraction) -> Option<String> {
    let end = if let Some(body) = node.child_by_field_name("body") {
        body.start_byte()
    } else if node.kind() == "macro_definition_v2" {
        node.child_by_field_name("rules")
            .map_or_else(|| node.end_byte(), |rules| rules.start_byte())
    } else if node.kind() == "macro_definition" {
        let mut macro_end = node.end_byte();
        if let Some(pos) = macro_body_start(node) {
            macro_end = pos;
        }
        macro_end
    } else {
        node.end_byte()
    };
    let start = declaration_start_node(node).start_byte();
    match node_text(source, start, end) {
        Ok(raw) => {
            let trimmed = if node.kind() == "function_signature_item" {
                raw.trim_end().trim_end_matches(';').trim_end()
            } else {
                raw.trim_end()
            };
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Err(error) => {
            tracing::warn!(%error, "Rust signature skipped; extraction is partial");
            out.truncated = true;
            None
        }
    }
}

fn macro_body_start(node: Node<'_>) -> Option<usize> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if (!child.is_named() && matches!(child.kind(), "{" | "(" | "["))
            || child.kind() == "macro_rule"
        {
            return Some(child.start_byte());
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

fn emit_structure(
    node: Node<'_>,
    (kind, name, visibility, rust_mod_path): (
        &'static str,
        Option<String>,
        Option<String>,
        Option<RustModPathPayload>,
    ),
    meta: &NodeMeta<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let Some(next) = counters.structure.checked_add(1) else {
        out.truncated = true;
        return Ok(());
    };
    let decorators = meta.decorator_strings(input.source, out);
    let signature = item_signature(node, input.source, out);
    let key = format!("structure:{}", counters.structure);
    counters.structure = next;
    out.items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: declaration_span(node, input.lines, "Rust structure")?,
        payload: EvidencePayload::Structure(StructurePayload {
            kind: kind.to_owned(),
            name,
            visibility,
            decorators,
            signature,
            doc_comment: meta.outer_doc.clone(),
            rust_mod_path,
        }),
    });
    Ok(())
}

fn emit_pack_symbol(
    node: Node<'_>,
    (name, kind, visibility, type_annotation, doc): (
        String,
        &'static str,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let Some(next) = counters.symbol.checked_add(1) else {
        out.truncated = true;
        return Ok(());
    };
    let key = format!("symbol:{}", counters.symbol);
    counters.symbol = next;
    out.items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Symbol,
        origin: EvidenceOrigin::PackSymbol,
        span: declaration_span(node, input.lines, "Rust symbol")?,
        payload: EvidencePayload::Symbol(SymbolPayload {
            name,
            kind: kind.to_owned(),
            visibility,
            type_annotation,
            doc,
        }),
    });
    Ok(())
}

fn emit_import(
    node: Node<'_>,
    inline_mods: &InlineModStack,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let start = declaration_start_node(node).start_byte();
    let text = match node_text(input.source, start, node.end_byte()) {
        Ok(text) => text.trim().to_owned(),
        Err(error) => {
            tracing::warn!(%error, "Rust import skipped; extraction is partial");
            out.truncated = true;
            return Ok(());
        }
    };
    let Some(next) = counters.import.checked_add(1) else {
        out.truncated = true;
        return Ok(());
    };
    let is_wildcard = node.kind() == "use_declaration" && has_use_wildcard(node, out);
    let key = format!("import:{}", counters.import);
    counters.import = next;
    out.items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: declaration_span(node, input.lines, "Rust import")?,
        payload: EvidencePayload::Import(ImportPayload {
            source: text,
            items: Vec::new(),
            alias: None,
            is_wildcard,
            inline_segments: inline_mods.segments().to_vec(),
        }),
    });
    Ok(())
}

fn has_use_wildcard(root: Node<'_>, out: &mut Extraction) -> bool {
    let mut pending = vec![(root, 0u32)];
    while let Some((node, depth)) = pending.pop() {
        if depth >= super::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        if node.kind() == "use_wildcard" {
            return true;
        }
        if let Some(next_depth) = depth.checked_add(1) {
            super::push_children(&mut pending, node, |child| (child, next_depth));
        }
    }
    false
}

fn emit_comment(
    node: Node<'_>,
    next_sibling: Option<Node<'_>>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let text = match node_text(input.source, node.start_byte(), node.end_byte()) {
        Ok(text) => text.to_owned(),
        Err(error) => {
            tracing::warn!(%error, "Rust comment skipped; extraction is partial");
            out.truncated = true;
            return Ok(());
        }
    };
    let Some(next_idx) = counters.comment.checked_add(1) else {
        out.truncated = true;
        return Ok(());
    };
    let end_row = content_end_row(node, input.source);
    let associated_node = next_sibling.and_then(|sib| {
        let sib_row = declaration_start_node(sib).start_position().row;
        end_row
            .checked_add(1)
            .is_some_and(|adj| sib_row <= adj)
            .then(|| sib.kind().to_owned())
    });
    let comment_kind = classify_rust_comment(node.kind(), &text);
    let key = format!("comment:{}", counters.comment);
    counters.comment = next_idx;
    out.items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Comment,
        origin: EvidenceOrigin::PackStructure,
        span: node_span(node, input.lines, "Rust comment")?,
        payload: EvidencePayload::Comment(CommentPayload {
            text,
            kind: comment_kind.to_owned(),
            associated_node,
        }),
    });
    Ok(())
}

fn emit_diagnostic(
    node: Node<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counters: &mut Counters,
) -> Result<(), PositionOverflow> {
    let message = if node.is_missing() {
        format!("Missing expected node: {}", node.kind())
    } else {
        match node_text(input.source, node.start_byte(), node.end_byte()) {
            Ok(slice) => format!("Syntax error: unexpected '{slice}'"),
            Err(_) => "Syntax error".to_owned(),
        }
    };
    let Some(next) = counters.diagnostic.checked_add(1) else {
        out.truncated = true;
        return Ok(());
    };
    let key = format!("diagnostic:{}", counters.diagnostic);
    counters.diagnostic = next;
    out.items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::PackStructure,
        span: node_span(node, input.lines, "Rust diagnostic")?,
        payload: EvidencePayload::Diagnostic(DiagnosticPayload {
            message,
            severity: "error".to_owned(),
            original_item_count: None,
            kept_item_count: None,
        }),
    });
    Ok(())
}

#[cfg(test)]
#[path = "rust_items_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rust_items_orchard_tests.rs"]
mod orchard_tests;

#[cfg(test)]
#[path = "rust_imports_tests.rs"]
mod imports_tests;

#[cfg(test)]
#[path = "rust_pattern_vars_tests.rs"]
mod pattern_vars_tests;
