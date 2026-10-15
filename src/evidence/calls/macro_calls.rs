//! Recover executable call expressions hidden by Rust macro token-tree parsing.
//!
//! Reparse a macro's argument tokens as a synthetic *function argument list*.
//! This lets Tree-sitter recognize nested method/function calls without
//! treating string contents or comments as calls. All coordinates are mapped
//! back to the original source; the synthetic probe is never emitted.
use super::{callee_node, enclosing_caller, named_children, simple_name};
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, Position, Span, SpanAvailability, char_column,
    saturating_u32,
};
use serde_json::json;
use tree_sitter_language_pack::{Node, get_parser};

const PREFIX: &str = "fn __slug_macro() { __slug_probe(";
const SUFFIX: &str = "); }";

pub(super) fn extract(macro_node: &Node, source: &str) -> Vec<(usize, usize, EvidenceItem)> {
    let Some(tokens) = named_children(macro_node).find(|n| n.kind() == "token_tree") else {
        return Vec::new();
    };
    let start = tokens.start_byte().saturating_add(1);
    let end = tokens.end_byte().saturating_sub(1);
    let mut rows = Vec::new();
    scan_fragment(
        source,
        start,
        end,
        enclosing_caller(macro_node, source),
        &mut rows,
    );
    rows
}

fn scan_fragment(
    source: &str,
    start: usize,
    end: usize,
    caller: Option<String>,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
) {
    let Some(body) = source.get(start..end) else {
        return;
    };
    let wrapped = format!("{PREFIX}{body}{SUFFIX}");
    let Ok(mut parser) = get_parser("rust") else {
        return;
    };
    let Some(tree) = parser.parse(&wrapped) else {
        return;
    };
    collect(tree.root_node(), &wrapped, source, start, end, caller, rows);
}

fn collect(
    node: Node,
    wrapped: &str,
    source: &str,
    source_start: usize,
    source_end: usize,
    caller: Option<String>,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
) {
    if matches!(node.kind().as_str(), "call_expression" | "macro_invocation") {
        let macro_call = node.kind() == "macro_invocation";
        let relative_start = node.start_byte();
        let relative_end = node.end_byte();
        if relative_start >= PREFIX.len()
            && relative_end <= wrapped.len().saturating_sub(SUFFIX.len())
        {
            let start = source_start + relative_start - PREFIX.len();
            let end = source_start + relative_end - PREFIX.len();
            if end <= source_end
                && let Some(callee) = callee_node(&node, macro_call)
            {
                let callee_text = &wrapped[callee.start_byte()..callee.end_byte()];
                let name = simple_name(callee, wrapped)
                    .unwrap_or_else(|| callee_text.trim_end_matches('!').to_owned());
                let resolution = if macro_call {
                    "macro"
                } else if callee_text.contains("::") {
                    "qualified-syntactic"
                } else {
                    "unresolved-syntactic"
                };
                let span = Span::new(
                    start as u64,
                    end as u64,
                    position(source, start),
                    position(source, end),
                )
                .map_or_else(
                    |_| SpanAvailability::NormalizerUnavailable {
                        reason: "macro argument call span invalid".into(),
                    },
                    SpanAvailability::Present,
                );
                rows.push((
                    start,
                    end,
                    EvidenceItem {
                        key: String::new(),
                        kind: EvidenceKind::Call,
                        origin: EvidenceOrigin::RustCallWalker,
                        span,
                        payload: json!({
                            "language": "rust",
                            "callee_text": callee_text,
                            "callee_name": name,
                            "caller": caller,
                            "resolution": resolution,
                            "confidence": if macro_call { "high" } else { "medium" },
                        }),
                    },
                ));
            }
        }
    }
    // A nested macro still owns an opaque token tree after the outer
    // fragment is reparsed. Recurse into its contents using absolute source
    // coordinates, preserving the enclosing Rust function as caller.
    if node.kind() == "macro_invocation"
        && node.start_byte() >= PREFIX.len()
        && node.end_byte() <= wrapped.len().saturating_sub(SUFFIX.len())
        && let Some(tokens) = named_children(&node).find(|n| n.kind() == "token_tree")
    {
        let start = source_start + tokens.start_byte() + 1 - PREFIX.len();
        let end = source_start + tokens.end_byte().saturating_sub(1) - PREFIX.len();
        if end <= source_end && start < end {
            scan_fragment(source, start, end, caller.clone(), rows);
        }
    }
    for child in named_children(&node) {
        collect(
            child,
            wrapped,
            source,
            source_start,
            source_end,
            caller.clone(),
            rows,
        );
    }
}

fn position(source: &str, byte: usize) -> Position {
    let prefix = &source[..byte];
    Position {
        line: saturating_u32(prefix.bytes().filter(|b| *b == b'\n').count()),
        column: char_column(source, byte),
    }
}
