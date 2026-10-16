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
    collect(
        tree.root_node(),
        &wrapped,
        source,
        (start, end),
        caller,
        rows,
        0,
    );
}

/// `source_start`/`source_end` arrive as one tuple: they are the fragment's
/// coordinate frame within `source`, meaningless apart, and grouping them is
/// what keeps `depth` from pushing the parameter list past the limit the
/// lint — and a reader — can hold.
fn collect(
    node: Node,
    wrapped: &str,
    source: &str,
    (source_start, source_end): (usize, usize),
    caller: Option<String>,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
    depth: u32,
) -> bool {
    // The depth guard bounds descent within THIS synthetic tree only;
    // nested macros re-enter `scan_fragment` at depth 0 by design (see
    // `recurse_into_nested_macro`) — the macro-nesting chain is separately
    // bounded by the token trees it reparses.
    if depth >= crate::evidence::MAX_WALK_DEPTH {
        return true;
    }
    // Emit the node's own row *before* descending, so `calls::walk`'s
    // first-wins dedup keeps the outermost row.
    if matches!(node.kind().as_str(), "call_expression" | "macro_invocation")
        && within_synthetic_frame(&node, wrapped)
        && let Some(row) = emit_call_row(&node, wrapped, source, source_start, source_end, &caller)
    {
        rows.push(row);
    }
    recurse_into_nested_macro(
        &node,
        wrapped,
        source,
        source_start,
        source_end,
        &caller,
        rows,
    );
    let mut capped = false;
    for child in named_children(&node) {
        capped |= collect(
            child,
            wrapped,
            source,
            (source_start, source_end),
            caller.clone(),
            rows,
            depth + 1,
        );
    }
    capped
}

/// True when the node's byte range sits inside the probe body of the
/// synthetic frame (not in the `PREFIX`/`SUFFIX` scaffolding itself).
fn within_synthetic_frame(node: &Node, wrapped: &str) -> bool {
    node.start_byte() >= PREFIX.len()
        && node.end_byte() <= wrapped.len().saturating_sub(SUFFIX.len())
}

/// Builds one call row for a `call_expression`/`macro_invocation` node mapped
/// back to original-source coordinates, or `None` when the node is the probe
/// wrapper itself, has no callee, or falls outside the fragment.
fn emit_call_row(
    node: &Node,
    wrapped: &str,
    source: &str,
    source_start: usize,
    source_end: usize,
    caller: &Option<String>,
) -> Option<(usize, usize, EvidenceItem)> {
    let macro_call = node.kind() == "macro_invocation";
    let start = source_start + node.start_byte() - PREFIX.len();
    let end = source_start + node.end_byte() - PREFIX.len();
    if end > source_end {
        return None;
    }
    let callee = callee_node(node, macro_call)?;
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
    Some((
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
    ))
}

/// A nested macro still owns an opaque token tree after the outer fragment is
/// reparsed. Recurse into its contents using absolute source coordinates,
/// preserving the enclosing Rust function as caller.
fn recurse_into_nested_macro(
    node: &Node,
    wrapped: &str,
    source: &str,
    source_start: usize,
    source_end: usize,
    caller: &Option<String>,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
) {
    if node.kind() != "macro_invocation" || !within_synthetic_frame(node, wrapped) {
        return;
    }
    let Some(tokens) = named_children(node).find(|n| n.kind() == "token_tree") else {
        return;
    };
    let start = source_start + tokens.start_byte() + 1 - PREFIX.len();
    let end = source_start + tokens.end_byte().saturating_sub(1) - PREFIX.len();
    if end <= source_end && start < end {
        scan_fragment(source, start, end, caller.clone(), rows);
    }
}

fn position(source: &str, byte: usize) -> Position {
    let prefix = &source[..byte];
    Position {
        line: saturating_u32(prefix.bytes().filter(|b| *b == b'\n').count()),
        column: char_column(source, byte),
    }
}
