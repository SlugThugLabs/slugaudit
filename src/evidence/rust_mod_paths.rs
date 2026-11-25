//! Enriches Rust `Structure` `Module` evidence items with `#[path = "..."]`
//! metadata (`rust_mod_path`) for external module declarations.

use super::rust_path_attr::parse_attribute_item;
use super::{Extraction, WalkInput, named_children, node_text, reverse_from};
use crate::model::{EvidenceItem, EvidenceKind, SpanAvailability};
use serde_json::{Value, json};
use tree_sitter_language_pack::{Node, Tree};

type PendingStep = Option<(Node, u32, Vec<Node>)>;

struct WalkState<'a> {
    source: &'a str,
    items: &'a mut [EvidenceItem],
    inline_stack: Vec<String>,
    pending: Vec<PendingStep>,
    out: &'a mut Extraction,
}

/// Walks `tree` iteratively and attaches `rust_mod_path` to external Rust
/// `mod_item` `Structure` items in `items` that carry `#[path = "..."]` or
/// `#[cfg_attr(..., path = "...")]`.
pub(super) fn enrich_rust_mod_paths(
    tree: &Tree,
    input: &WalkInput<'_>,
    items: &mut [EvidenceItem],
    out: &mut Extraction,
) {
    let mut state = WalkState {
        source: input.source,
        items,
        inline_stack: Vec::new(),
        pending: vec![Some((tree.root_node(), 0, Vec::new()))],
        out,
    };

    while let Some(step) = state.pending.pop() {
        let Some((node, depth, attrs)) = step else {
            state.inline_stack.pop();
            continue;
        };
        if super::deadline_spent(input.deadline) {
            state.out.timed_out = true;
            break;
        }
        if depth >= super::MAX_WALK_DEPTH {
            state.out.truncated = true;
            continue;
        }
        let Some(child_depth) = depth.checked_add(1) else {
            state.out.truncated = true;
            continue;
        };
        if node.kind() != "mod_item" {
            push_grouped_children(&mut state.pending, &node, child_depth);
            continue;
        }
        process_mod_item(&mut state, &node, attrs, child_depth);
    }
}

fn process_mod_item(
    state: &mut WalkState<'_>,
    node: &Node,
    mut attrs: Vec<Node>,
    child_depth: u32,
) {
    let mod_name = node
        .child_by_field_name("name")
        .and_then(|child| slice_node_text(state.source, &child, state.out));
    attrs.extend(named_children(node).filter(|child| child.kind() == "attribute_item"));
    let path_attr = attrs
        .iter()
        .filter_map(|attr| {
            slice_node_text(state.source, attr, state.out).and_then(parse_attribute_item)
        })
        .last();

    if named_children(node).any(|child| child.kind() == "declaration_list") {
        let segment = path_attr.or_else(|| {
            mod_name.map(|name| {
                name.strip_prefix("r#")
                    .map_or(name, |stripped| stripped)
                    .to_owned()
            })
        });
        if let Some(segment) = segment {
            state.inline_stack.push(segment);
            state.pending.push(None);
        }
        push_grouped_children(&mut state.pending, node, child_depth);
    } else if let (Some(path_value), Some(name)) = (path_attr, mod_name) {
        attach_rust_mod_path(state.items, node, name, &path_value, &state.inline_stack);
    }
}

fn slice_node_text<'a>(source: &'a str, node: &Node, out: &mut Extraction) -> Option<&'a str> {
    match node_text(source, node.start_byte(), node.end_byte()) {
        Ok(text) => Some(text),
        Err(error) => {
            tracing::warn!(%error, "Rust mod node skipped; extraction is partial");
            out.truncated = true;
            None
        }
    }
}

fn push_grouped_children(pending: &mut Vec<PendingStep>, node: &Node, child_depth: u32) {
    let first = pending.len();
    let mut pending_attrs = Vec::new();
    for child in named_children(node) {
        match child.kind().as_str() {
            "attribute_item" => pending_attrs.push(child),
            "line_comment" | "block_comment" => {
                // Comments between an attribute and its target item do not detach the attribute.
            }
            "mod_item" => {
                pending.push(Some((
                    child,
                    child_depth,
                    std::mem::take(&mut pending_attrs),
                )));
            }
            _ => {
                pending_attrs.clear();
                pending.push(Some((child, child_depth, Vec::new())));
            }
        }
    }
    reverse_from(pending, first);
}

fn attach_rust_mod_path(
    items: &mut [EvidenceItem],
    mod_node: &Node,
    mod_name: &str,
    path_attr: &str,
    inline_stack: &[String],
) {
    let mod_start = mod_node.start_byte() as u64;
    let mod_end = mod_node.end_byte() as u64;
    let Some(item) = items.iter_mut().find(|item| {
        item.kind == EvidenceKind::Structure
            && item.payload.get("kind").and_then(Value::as_str) == Some("Module")
            && item.payload.get("name").and_then(Value::as_str) == Some(mod_name)
            && matches!(
                &item.span,
                SpanAvailability::Present(span)
                    if span.start_byte == mod_start && span.end_byte == mod_end
            )
    }) else {
        return;
    };
    if let Some(map) = item.payload.as_object_mut() {
        map.insert(
            "rust_mod_path".to_owned(),
            json!({
                "path_attr": path_attr,
                "inline_dir": inline_stack.join("/"),
                "inline_depth": inline_stack.len(),
            }),
        );
    }
}

#[cfg(test)]
#[path = "rust_mod_paths_tests.rs"]
mod tests;
