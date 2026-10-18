//! Rust call-site evidence.
//!
//! This module deliberately reports syntax facts, not a compiler-grade call
//! graph. A Tree-sitter call node proves that source contains a call-shaped
//! expression and gives us its span; name resolution can still be ambiguous
//! for traits, generics, function pointers, macros, and dynamic dispatch.

use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, LineIndex, Position, Span, SpanAvailability,
    saturating_u32,
};
use serde_json::json;
use std::collections::HashSet;

mod macro_calls;
use super::{TextRangeError, node_text};
use tree_sitter_language_pack::Node;

const RUST_CALL_KINDS: &[&str] = &["call_expression", "macro_invocation"];
const RUST_FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_signature_item",
    "closure_expression",
    "impl_item",
    "trait_item",
];

/// Extracts Rust call-site evidence from one already indexed source file.
/// The flag is true when the extraction is truncated, which the caller must
/// report as partial: a walk bound (tree depth or macro nesting) was hit, the
/// parser could not produce a tree, or a node range did not slice the source.
pub(crate) fn extract_rust(source: &str, lines: &LineIndex<'_>) -> (Vec<EvidenceItem>, bool) {
    let Some(tree) = super::parse_tree("rust", source) else {
        return (Vec::new(), true);
    };
    let mut items = Vec::new();
    let mut counter = 0usize;
    let mut seen = HashSet::new();
    let capped = walk(
        tree.root_node(),
        source,
        lines,
        &mut items,
        &mut counter,
        &mut seen,
        0,
    );
    (items, capped)
}

/// The text-derived fields of one call node.
struct CallParts {
    callee_text: String,
    callee_name: String,
    caller: Option<String>,
}

fn call_parts(node: &Node, source: &str, macro_call: bool) -> Result<CallParts, TextRangeError> {
    let callee = callee_node(node, macro_call);
    let callee_text = match &callee {
        Some(child) => node_text(source, child.start_byte(), child.end_byte())?,
        None => node_text(source, node.start_byte(), node.end_byte())?,
    }
    .to_owned();
    let callee_name = match callee {
        Some(child) => simple_name(child, source)?,
        None => None,
    }
    .unwrap_or_else(|| callee_text.trim_end_matches('!').to_owned());
    Ok(CallParts {
        callee_text,
        callee_name,
        caller: enclosing_caller(node, source)?,
    })
}

fn walk(
    node: Node,
    source: &str,
    lines: &LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
    seen: &mut HashSet<(usize, usize)>,
    depth: u32,
) -> bool {
    if depth >= crate::evidence::MAX_WALK_DEPTH {
        return true;
    }
    let mut capped = false;
    if RUST_CALL_KINDS.contains(&node.kind().as_str()) {
        let macro_call = node.kind() == "macro_invocation";
        match call_parts(&node, source, macro_call) {
            Ok(parts) => capped |= emit_call(&node, parts, source, lines, items, counter, seen),
            Err(error) => {
                tracing::warn!(%error, "Rust call node skipped; extraction is partial");
                capped = true;
            }
        }
    }
    for child in named_children(&node) {
        capped |= walk(child, source, lines, items, counter, seen, depth + 1);
    }
    capped
}

/// Pushes the call row for `node` (and, for a macro, the calls recovered from
/// its token tree). Returns true when the macro scan was truncated.
fn emit_call(
    node: &Node,
    parts: CallParts,
    source: &str,
    lines: &LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    counter: &mut usize,
    seen: &mut HashSet<(usize, usize)>,
) -> bool {
    let macro_call = node.kind() == "macro_invocation";
    let (resolution, confidence) = if macro_call {
        ("macro", "high")
    } else if parts.callee_text.contains("::") {
        ("qualified-syntactic", "high")
    } else {
        ("unresolved-syntactic", "medium")
    };
    let key = format!("call:{counter}");
    *counter += 1;
    if seen.insert((node.start_byte(), node.end_byte())) {
        items.push(EvidenceItem {
            key,
            kind: EvidenceKind::Call,
            origin: EvidenceOrigin::RustCallWalker,
            span: node_span(node, lines),
            payload: json!({
                "language": "rust",
                "callee_text": parts.callee_text,
                "callee_name": parts.callee_name,
                "caller": parts.caller,
                "resolution": resolution,
                "confidence": confidence,
            }),
        });
    }
    if !macro_call {
        return false;
    }
    let recovered = macro_calls::extract(node, source, lines);
    for (start, end, mut item) in recovered.rows {
        if seen.insert((start, end)) {
            item.key = format!("call:{counter}");
            *counter += 1;
            items.push(item);
        }
    }
    recovered.capped
}

fn callee_node(node: &Node, macro_call: bool) -> Option<Node> {
    named_children(node).find(|child| {
        macro_call
            || child.kind() == "identifier"
            || child.kind() == "scoped_identifier"
            || child.kind() == "field_expression"
    })
}

/// The simple name of a callee. `field_expression` (`v.unwrap`) recurses to
/// its last named child — the `field_identifier` — so a method call reports
/// `unwrap`, while `callee_text` keeps the receiver for context.
fn simple_name(node: Node, source: &str) -> Result<Option<String>, TextRangeError> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return Ok(Some(
            node_text(source, node.start_byte(), node.end_byte())?.to_owned(),
        ));
    }
    match named_children(&node).last() {
        Some(child) => simple_name(child, source),
        None => Ok(None),
    }
}

fn enclosing_caller(node: &Node, source: &str) -> Result<Option<String>, TextRangeError> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if RUST_FUNCTION_KINDS.contains(&parent.kind().as_str()) {
            return function_name(&parent, source);
        }
        current = parent.parent();
    }
    Ok(None)
}

fn function_name(node: &Node, source: &str) -> Result<Option<String>, TextRangeError> {
    match named_children(node).find(|child| child.kind() == "identifier") {
        Some(child) => Ok(Some(
            node_text(source, child.start_byte(), child.end_byte())?.to_owned(),
        )),
        None => Ok(None),
    }
}

fn named_children(node: &Node) -> impl Iterator<Item = Node> {
    (0..node.named_child_count() as u32).filter_map(move |index| node.named_child(index))
}

fn node_span(node: &Node, lines: &LineIndex<'_>) -> SpanAvailability {
    let start = Position {
        line: saturating_u32(node.start_position().row),
        column: lines.column(node.start_byte()),
    };
    let end = Position {
        line: saturating_u32(node.end_position().row),
        column: lines.column(node.end_byte()),
    };
    Span::new(node.start_byte() as u64, node.end_byte() as u64, start, end).map_or_else(
        |error| super::span_unavailable("Rust call", error),
        SpanAvailability::Present,
    )
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
