//! Rust call-site evidence.
//!
//! This module deliberately reports syntax facts, not a compiler-grade call
//! graph. A Tree-sitter call node proves that source contains a call-shaped
//! expression and gives us its span; name resolution can still be ambiguous
//! for traits, generics, function pointers, macros, and dynamic dispatch.

use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, Position, Span, SpanAvailability, char_column,
    saturating_u32,
};
use serde_json::json;
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
/// Returns an empty vector for non-Rust input or when the parser cannot
/// produce a tree.
pub(crate) fn extract_rust(source: &str) -> Vec<EvidenceItem> {
    let Ok(mut parser) = tree_sitter_language_pack::get_parser("rust") else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(source) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut counter = 0usize;
    walk(tree.root_node(), source, &mut items, &mut counter);
    items
}

fn walk(node: Node, source: &str, items: &mut Vec<EvidenceItem>, counter: &mut usize) {
    if RUST_CALL_KINDS.contains(&node.kind().as_str()) {
        let macro_call = node.kind() == "macro_invocation";
        let callee = callee_node(&node, macro_call);
        let callee_text = callee
            .as_ref()
            .map(|child| source[child.start_byte()..child.end_byte()].to_owned())
            .unwrap_or_else(|| source[node.start_byte()..node.end_byte()].to_owned());
        let callee_name = callee
            .and_then(|child| simple_name(child, source))
            .unwrap_or_else(|| callee_text.trim_end_matches('!').to_owned());
        let caller = enclosing_caller(&node, source);
        let (resolution, confidence) = if macro_call {
            ("macro", "high")
        } else if callee_text.contains("::") {
            ("qualified-syntactic", "high")
        } else {
            ("unresolved-syntactic", "medium")
        };
        let key = format!("call:{counter}");
        *counter += 1;
        items.push(EvidenceItem {
            key,
            kind: EvidenceKind::Call,
            origin: EvidenceOrigin::RustCallWalker,
            span: node_span(&node, source),
            payload: json!({
                "language": "rust",
                "callee_text": callee_text,
                "callee_name": callee_name,
                "caller": caller,
                "resolution": resolution,
                "confidence": confidence,
            }),
        });
    }
    for child in named_children(&node) {
        walk(child, source, items, counter);
    }
}

fn callee_node(node: &Node, macro_call: bool) -> Option<Node> {
    named_children(node).find(|child| {
        macro_call || child.kind() == "identifier" || child.kind() == "scoped_identifier"
    })
}

fn simple_name(node: Node, source: &str) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(source[node.start_byte()..node.end_byte()].to_owned());
    }
    named_children(&node)
        .last()
        .and_then(|child| simple_name(child, source))
}

fn enclosing_caller(node: &Node, source: &str) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if RUST_FUNCTION_KINDS.contains(&parent.kind().as_str()) {
            return function_name(&parent, source);
        }
        current = parent.parent();
    }
    None
}

fn function_name(node: &Node, source: &str) -> Option<String> {
    named_children(node).find_map(|child| {
        if child.kind() == "identifier" {
            Some(source[child.start_byte()..child.end_byte()].to_owned())
        } else {
            None
        }
    })
}

fn named_children(node: &Node) -> impl Iterator<Item = Node> {
    (0..node.named_child_count() as u32).filter_map(move |index| node.named_child(index))
}

fn node_span(node: &Node, source: &str) -> SpanAvailability {
    let start = Position {
        line: saturating_u32(node.start_position().row),
        column: char_column(source, node.start_byte()),
    };
    let end = Position {
        line: saturating_u32(node.end_position().row),
        column: char_column(source, node.end_byte()),
    };
    Span::new(node.start_byte() as u64, node.end_byte() as u64, start, end).map_or_else(
        |_| SpanAvailability::NormalizerUnavailable {
            reason: "Rust call span failed local range validation".into(),
        },
        SpanAvailability::Present,
    )
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
