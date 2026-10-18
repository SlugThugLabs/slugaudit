//! Recover executable call expressions hidden by Rust macro token-tree parsing.
//!
//! Reparse a macro's argument tokens as a synthetic *function argument list*.
//! This lets Tree-sitter recognize nested method/function calls without
//! treating string contents or comments as calls. All coordinates are mapped
//! back to the original source; the synthetic probe is never emitted.
use super::{callee_node, enclosing_caller, named_children, simple_name};
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, LineIndex, Span, SpanAvailability};
use serde_json::json;
use tree_sitter_language_pack::{Node, get_parser};

const PREFIX: &str = "fn __slug_macro() { __slug_probe(";
const SUFFIX: &str = "); }";

/// How many macro invocations may nest inside one another before the
/// extraction stops reparsing. Each level reparses its whole body and
/// re-enters this module's recursion, so a file of `a!(a!(a!(...)))` costs
/// stack proportional to the nesting and parse work quadratic in it —
/// 10 000 levels abort the process with a stack overflow. Real code nests a
/// handful of levels; 64 is far past that. Past it the remaining fragment is
/// not scanned and the caller is told, so the file is reported `Partial`.
pub(super) const MAX_MACRO_DEPTH: u32 = 64;

/// Calls recovered from one macro, and whether any bound truncated the scan.
pub(super) struct MacroCalls {
    pub(super) rows: Vec<(usize, usize, EvidenceItem)>,
    pub(super) capped: bool,
}

/// What stays fixed across every fragment of one outermost macro.
struct Scan<'a> {
    source: &'a str,
    lines: &'a LineIndex<'a>,
    caller: Option<String>,
}

/// One reparsed fragment: the synthetic text and its frame in the source.
struct Fragment<'a> {
    wrapped: &'a str,
    start: usize,
    end: usize,
}

pub(super) fn extract(macro_node: &Node, source: &str, lines: &LineIndex<'_>) -> MacroCalls {
    let mut out = MacroCalls {
        rows: Vec::new(),
        capped: false,
    };
    let Some(tokens) = named_children(macro_node).find(|n| n.kind() == "token_tree") else {
        return out;
    };
    let start = tokens.start_byte().saturating_add(1);
    let end = tokens.end_byte().saturating_sub(1);
    let scan = Scan {
        source,
        lines,
        caller: enclosing_caller(macro_node, source),
    };
    out.capped = scan_fragment(&scan, (start, end), 0, &mut out.rows);
    out
}

/// Returns true when the macro-nesting or tree-depth bound truncated the scan.
fn scan_fragment(
    scan: &Scan<'_>,
    (start, end): (usize, usize),
    macro_depth: u32,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
) -> bool {
    if macro_depth >= MAX_MACRO_DEPTH {
        return true;
    }
    let Some(body) = scan.source.get(start..end) else {
        return false;
    };
    let wrapped = format!("{PREFIX}{body}{SUFFIX}");
    let Ok(mut parser) = get_parser("rust") else {
        return false;
    };
    let Some(tree) = parser.parse(&wrapped) else {
        return false;
    };
    let fragment = Fragment {
        wrapped: &wrapped,
        start,
        end,
    };
    collect(tree.root_node(), scan, &fragment, macro_depth, rows, 0)
}

fn collect(
    node: Node,
    scan: &Scan<'_>,
    fragment: &Fragment<'_>,
    macro_depth: u32,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
    depth: u32,
) -> bool {
    // The depth guard bounds descent within THIS synthetic tree only; the
    // chain of nested macros is bounded separately by `MAX_MACRO_DEPTH`,
    // which `recurse_into_nested_macro` threads through every re-entry.
    if depth >= crate::evidence::MAX_WALK_DEPTH {
        return true;
    }
    // Emit the node's own row *before* descending, so `calls::walk`'s
    // first-wins dedup keeps the outermost row.
    if matches!(node.kind().as_str(), "call_expression" | "macro_invocation")
        && within_synthetic_frame(&node, fragment.wrapped)
        && let Some(row) = emit_call_row(&node, scan, fragment)
    {
        rows.push(row);
    }
    let mut capped = recurse_into_nested_macro(&node, scan, fragment, macro_depth, rows);
    for child in named_children(&node) {
        capped |= collect(child, scan, fragment, macro_depth, rows, depth + 1);
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
    scan: &Scan<'_>,
    fragment: &Fragment<'_>,
) -> Option<(usize, usize, EvidenceItem)> {
    let wrapped = fragment.wrapped;
    let macro_call = node.kind() == "macro_invocation";
    let start = fragment.start + node.start_byte() - PREFIX.len();
    let end = fragment.start + node.end_byte() - PREFIX.len();
    if end > fragment.end {
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
        scan.lines.position(start),
        scan.lines.position(end),
    )
    .map_or_else(
        |error| crate::evidence::span_unavailable("macro argument call", error),
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
                "caller": scan.caller,
                "resolution": resolution,
                "confidence": if macro_call { "high" } else { "medium" },
            }),
        },
    ))
}

/// A nested macro still owns an opaque token tree after the outer fragment is
/// reparsed. Recurse into its contents using absolute source coordinates,
/// preserving the enclosing Rust function as caller. Returns true when the
/// nesting bound stopped a scan below this macro.
fn recurse_into_nested_macro(
    node: &Node,
    scan: &Scan<'_>,
    fragment: &Fragment<'_>,
    macro_depth: u32,
    rows: &mut Vec<(usize, usize, EvidenceItem)>,
) -> bool {
    if node.kind() != "macro_invocation" || !within_synthetic_frame(node, fragment.wrapped) {
        return false;
    }
    let Some(tokens) = named_children(node).find(|n| n.kind() == "token_tree") else {
        return false;
    };
    let start = fragment.start + tokens.start_byte() + 1 - PREFIX.len();
    let end = fragment.start + tokens.end_byte().saturating_sub(1) - PREFIX.len();
    if end <= fragment.end && start < end {
        return scan_fragment(scan, (start, end), macro_depth + 1, rows);
    }
    false
}
