//! Recover executable call expressions hidden by Rust macro token-tree parsing.
//!
//! Reparse a macro's argument tokens as a synthetic *function argument list*.
//! This lets Tree-sitter recognize nested method/function calls without
//! treating string contents or comments as calls. All coordinates are mapped
//! back to the original source; the synthetic probe is never emitted.
use super::{
    CallSite, Caller, call_item, callee_node, caller_below, caller_text, named_children,
    qualified_callee, simple_name,
};
use crate::evidence::{NoTree, NodeError, WalkInput, deadline_spent, node_text, parse_tree};
use crate::model::{EvidenceItem, EvidenceOrigin, PositionOverflow};
use std::rc::Rc;
use tree_sitter_language_pack::Node;

const PREFIX: &str = "fn __slug_macro() { __slug_probe(";
const SUFFIX: &str = "); }";

/// How many macro invocations may nest inside one another before the
/// extraction stops reparsing. Each level reparses its whole body, so a file
/// of `a!(a!(a!(...)))` costs parse work quadratic in the nesting. The scan
/// itself is a loop over a heap stack (see [`scan`]), so nesting costs no
/// thread stack; this bound exists for the parse work. Real code nests a
/// handful of levels; 64 is far past that. Past it the remaining fragment is
/// not scanned and the caller is told, so the file is reported `Partial`.
pub(super) const MAX_MACRO_DEPTH: u32 = 64;

/// Calls recovered from one macro, whether any bound truncated the scan, and
/// whether a fragment's re-parse hit the deadline (its calls are missing).
#[derive(Default)]
pub(super) struct MacroCalls {
    pub(super) rows: Vec<(usize, usize, EvidenceItem)>,
    pub(super) capped: bool,
    pub(super) timed_out: bool,
}

/// One reparsed fragment: the synthetic text, its frame in the source, and
/// how many macros it sits inside. Shared by every pending node of its tree.
struct Fragment {
    wrapped: String,
    start: usize,
    end: usize,
    macro_depth: u32,
}

/// One node waiting on the scan stack, with its depth in its own fragment's
/// synthetic tree and the [`Caller`] its calls report.
struct Pending {
    node: Node,
    fragment: Rc<Fragment>,
    depth: u32,
    caller: Caller,
}

/// Recovers the calls in `macro_node`'s token tree. `caller` is the function
/// the macro sits in, which a call in the token tree reports unless a
/// closure (or other function-like node) inside the token tree encloses it,
/// exactly as outside a macro. Every fragment re-parse runs within what is
/// left of `input.deadline`, and the scan loop stops when it runs out. A
/// position past `u32::MAX` is an error that fails the file's extraction.
pub(super) fn extract(
    macro_node: &Node,
    input: &WalkInput<'_>,
    caller: Option<&str>,
) -> Result<MacroCalls, PositionOverflow> {
    let mut out = MacroCalls::default();
    let Some(tokens) = named_children(macro_node).find(|n| n.kind() == "token_tree") else {
        return Ok(out);
    };
    let start = tokens.start_byte().saturating_add(1);
    let end = tokens.end_byte().saturating_sub(1);
    scan(input, (start, end), Ok(caller.map(Rc::from)), &mut out)?;
    Ok(out)
}

/// Scans the outermost fragment and every macro nested in it, depth-first, on
/// one explicit heap stack: no recursion, so neither tree depth nor macro
/// nesting consumes thread stack. A node's own row is emitted before its
/// subtree and a nested macro's fragment is pushed on top of the node's
/// children, so rows come out in pre-order, as a recursive scan's would, and
/// `calls::walk`'s first-wins dedup keeps the outermost row.
/// Records in `out` whether the macro-nesting or tree-depth bound truncated
/// the scan, or the deadline ran out (a fragment's re-parse was cut, or the
/// scan loop stopped). Each pending node carries its [`Caller`], starting
/// from `caller`; a function-like node inside the frame (a closure) sets the
/// caller of its subtree, and a nested macro's fragment starts from the
/// caller in effect at that macro. A position past `u32::MAX` is an error
/// that fails the file's extraction.
fn scan(
    context: &WalkInput<'_>,
    range: (usize, usize),
    caller: Caller,
    out: &mut MacroCalls,
) -> Result<(), PositionOverflow> {
    let mut pending = Vec::new();
    open_fragment(context, (range, 0), caller, &mut pending, out);
    while let Some(Pending {
        node,
        fragment,
        depth,
        caller,
    }) = pending.pop()
    {
        if deadline_spent(context.deadline) {
            out.timed_out = true;
            break;
        }
        // Bounds descent within ONE synthetic tree; the chain of nested
        // macros is bounded separately by `MAX_MACRO_DEPTH`.
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.capped = true;
            continue;
        }
        let kind = node.kind();
        let in_frame = within_synthetic_frame(&node, &fragment.wrapped);
        if in_frame && matches!(kind.as_str(), "call_expression" | "macro_invocation") {
            match emit_call_row(&node, context, &fragment, &caller) {
                Ok(Some(row)) => out.rows.push(row),
                Ok(None) => {}
                Err(NodeError::Text(error)) => {
                    tracing::warn!(%error, "macro argument call skipped; extraction is partial");
                    out.capped = true;
                }
                Err(NodeError::Position(error)) => return Err(error),
            }
        }
        // The synthetic `fn __slug_macro` wrapper sits outside the frame, so
        // it never replaces the caller the scan started from.
        let below = if in_frame {
            caller_below(&node, &kind, &fragment.wrapped, &caller)
        } else {
            caller.clone()
        };
        crate::evidence::push_children(&mut pending, &node, |child| Pending {
            node: child,
            fragment: Rc::clone(&fragment),
            depth: depth + 1,
            caller: below.clone(),
        });
        if in_frame && let Some(nested) = nested_macro_range(&node, &fragment) {
            let nested = (nested, fragment.macro_depth + 1);
            open_fragment(context, nested, caller, &mut pending, out);
        }
    }
    Ok(())
}

/// Reparses `source[start..end]` inside the synthetic frame and pushes its
/// root, which reports `caller`, onto the scan stack. When the fragment
/// cannot be scanned it sets `out.capped` (`macro_depth` past
/// `MAX_MACRO_DEPTH`, an unsliceable range, or a failed parse) or
/// `out.timed_out` (the re-parse hit the file's deadline).
fn open_fragment(
    context: &WalkInput<'_>,
    ((start, end), macro_depth): ((usize, usize), u32),
    caller: Caller,
    pending: &mut Vec<Pending>,
    out: &mut MacroCalls,
) {
    if macro_depth >= MAX_MACRO_DEPTH {
        out.capped = true;
        return;
    }
    let body = match node_text(context.source, start, end) {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "macro argument fragment skipped; extraction is partial");
            out.capped = true;
            return;
        }
    };
    let wrapped = format!("{PREFIX}{body}{SUFFIX}");
    let tree = match parse_tree("rust", &wrapped, context.deadline) {
        Ok(tree) => tree,
        Err(why) => {
            out.timed_out |= why == NoTree::TimedOut;
            out.capped |= why == NoTree::Failed;
            return;
        }
    };
    pending.push(Pending {
        node: tree.root_node(),
        fragment: Rc::new(Fragment {
            wrapped,
            start,
            end,
            macro_depth,
        }),
        depth: 0,
        caller,
    });
}

/// True when the node's byte range sits inside the probe body of the
/// synthetic frame (not in the `PREFIX`/`SUFFIX` scaffolding itself).
fn within_synthetic_frame(node: &Node, wrapped: &str) -> bool {
    node.start_byte() >= PREFIX.len()
        && node.end_byte() <= wrapped.len().saturating_sub(SUFFIX.len())
}

/// Builds one call row for a `call_expression`/`macro_invocation` node mapped
/// back to original-source coordinates, or `None` when the node is the probe
/// wrapper itself, has no callee, or falls outside the fragment. An error
/// means a node range did not slice the synthetic text (or `enclosing` holds
/// such an error), or a position exceeds `u32::MAX`.
fn emit_call_row(
    node: &Node,
    input: &WalkInput<'_>,
    fragment: &Fragment,
    enclosing: &Caller,
) -> Result<Option<(usize, usize, EvidenceItem)>, NodeError> {
    let wrapped = fragment.wrapped.as_str();
    let macro_call = node.kind() == "macro_invocation";
    let start = fragment.start + node.start_byte() - PREFIX.len();
    let end = fragment.start + node.end_byte() - PREFIX.len();
    if end > fragment.end {
        return Ok(None);
    }
    let Some(callee) = callee_node(node, macro_call) else {
        return Ok(None);
    };
    let callee_text = node_text(wrapped, callee.start_byte(), callee.end_byte())?;
    let name = match simple_name(callee.clone(), wrapped)? {
        Some(name) => name,
        None => callee_text.trim_end_matches('!').to_owned(),
    };
    // The synthetic tree's rows belong to the wrapped text, so the original
    // source's positions come from the line index.
    let call_span = crate::evidence::checked_span(
        (start, end),
        input.lines.position(start)?,
        input.lines.position(end)?,
        "macro argument call",
    );
    let site = CallSite {
        language: "rust",
        origin: EvidenceOrigin::RustCallWalker,
        qualified: qualified_callee(&callee, wrapped)?,
        callee_text: callee_text.to_owned(),
        callee_name: name,
        caller: caller_text(enclosing)?,
        macro_call,
        span: call_span,
    };
    // The key is assigned by `calls::emit_call`, after its dedup.
    Ok(Some((start, end, call_item(String::new(), site))))
}

/// A nested macro still owns an opaque token tree after the outer fragment is
/// reparsed. Returns its contents' range in absolute source coordinates, so
/// the scan reparses it with the caller in effect at the macro, or
/// `None` when `node` is not a macro with a non-empty body inside the frame.
fn nested_macro_range(node: &Node, fragment: &Fragment) -> Option<(usize, usize)> {
    if node.kind() != "macro_invocation" {
        return None;
    }
    let tokens = named_children(node).find(|n| n.kind() == "token_tree")?;
    let start = fragment.start + tokens.start_byte() + 1 - PREFIX.len();
    let end = fragment.start + tokens.end_byte().saturating_sub(1) - PREFIX.len();
    (end <= fragment.end && start < end).then_some((start, end))
}
