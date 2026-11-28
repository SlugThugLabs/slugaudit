// slugaudit-line-exception: approved-by=split-judge; reason=Rust macro token-tree synthetic frame reparsing and offset-mapped call recovery
//! Recover executable call expressions hidden by Rust macro token-tree parsing.
//!
//! Reparses a macro's token-tree body in synthetic Rust frames (expression
//! argument list first, falling back to a function block and file-scope items
//! when the body holds statements or items). All coordinates are mapped back to
//! the original source; synthetic scaffolding is never emitted.

use super::{
    CallSite, Caller, call_item, callee_node, caller_below, caller_text, named_children,
    qualified_callee, simple_name,
};
use crate::evidence::{NodeError, WalkInput, deadline_spent, node_text};
use crate::model::{EvidenceItem, EvidenceOrigin, PositionOverflow};
use crate::parse::budget::{Parsed, parse_within};
use std::cmp::Reverse;
use std::rc::Rc;
use tree_sitter::{Language, Node, Parser, Tree};

/// How many macro invocations may nest inside one another before extraction
/// stops reparsing.
pub(super) const MAX_MACRO_DEPTH: u32 = 64;

#[derive(Clone, Copy)]
struct FrameSpec {
    prefix: &'static str,
    suffix: &'static str,
}

const EXPR_FRAME: FrameSpec = FrameSpec {
    prefix: "fn __slug_macro() { __slug_probe(",
    suffix: "\n); }",
};

const BLOCK_FRAME: FrameSpec = FrameSpec {
    prefix: "fn __slug_macro() {\n",
    suffix: "\n}",
};

const ITEM_FRAME: FrameSpec = FrameSpec {
    prefix: "",
    suffix: "",
};

const FRAMES: [FrameSpec; 3] = [EXPR_FRAME, BLOCK_FRAME, ITEM_FRAME];

struct Fragment {
    wrapped: String,
    prefix_len: usize,
    suffix_len: usize,
    start: usize,
    end: usize,
    macro_depth: u32,
}

impl Fragment {
    fn new(spec: FrameSpec, body: &str, (start, end): (usize, usize), macro_depth: u32) -> Self {
        Self {
            wrapped: format!("{}{body}{}", spec.prefix, spec.suffix),
            prefix_len: spec.prefix.len(),
            suffix_len: spec.suffix.len(),
            start,
            end,
            macro_depth,
        }
    }
}

/// Calls recovered from one macro, whether any bound truncated the scan, and
/// whether a fragment's re-parse hit the deadline.
#[derive(Default)]
pub(super) struct MacroCalls {
    pub(super) rows: Vec<(usize, usize, EvidenceItem)>,
    pub(super) capped: bool,
    pub(super) timed_out: bool,
}

struct FragmentJob {
    range: (usize, usize),
    macro_depth: u32,
    caller: Caller,
}

/// Recovers the calls in `macro_node`'s token tree, lazily initializing and
/// reusing `parser_slot` with `grammar` across macro invocations in one file.
pub(super) fn extract(
    macro_node: Node<'_>,
    input: &WalkInput<'_>,
    caller: Option<&str>,
    grammar: &Language,
    parser_slot: &mut Option<Parser>,
) -> Result<MacroCalls, PositionOverflow> {
    let mut out = MacroCalls::default();
    let Some(tokens) = named_children(macro_node).find(|n| n.kind() == "token_tree") else {
        return Ok(out);
    };
    let (start, end) = token_tree_bounds(tokens, input.source)?;
    if start >= end {
        return Ok(out);
    }
    if deadline_spent(input.deadline) {
        out.timed_out = true;
        return Ok(out);
    }
    if node_text(input.source, start, end).is_ok_and(|body| body.trim().is_empty()) {
        return Ok(out);
    }
    let parser = match parser_slot {
        Some(existing) => existing,
        slot @ None => {
            let mut fresh = Parser::new();
            if let Err(error) = fresh.set_language(grammar) {
                tracing::warn!(
                    language = "rust",
                    %error,
                    "evidence walker could not build a parser; extraction is partial"
                );
                out.capped = true;
                return Ok(out);
            }
            slot.insert(fresh)
        }
    };
    scan(
        input,
        (start, end),
        Ok(caller.map(Rc::from)),
        parser,
        &mut out,
    )?;
    out.rows
        .sort_by_key(|(row_start, row_end, _)| (*row_start, Reverse(*row_end)));
    Ok(out)
}

fn scan(
    context: &WalkInput<'_>,
    initial_range: (usize, usize),
    initial_caller: Caller,
    parser: &mut Parser,
    out: &mut MacroCalls,
) -> Result<(), PositionOverflow> {
    let mut jobs = vec![FragmentJob {
        range: initial_range,
        macro_depth: 0,
        caller: initial_caller,
    }];

    while let Some(job) = jobs.pop() {
        if deadline_spent(context.deadline) {
            out.timed_out = true;
            break;
        }
        if job.macro_depth >= MAX_MACRO_DEPTH {
            out.capped = true;
            continue;
        }
        let body = match node_text(context.source, job.range.0, job.range.1) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, "macro argument fragment skipped; extraction is partial");
                out.capped = true;
                continue;
            }
        };
        if body.trim().is_empty() {
            continue;
        }
        let Some((tree, fragment)) =
            parse_best_frame(body, job.range, job.macro_depth, context, parser, out)
        else {
            if out.timed_out {
                break;
            }
            continue;
        };
        walk_fragment(&tree, &fragment, context, job.caller, &mut jobs, out)?;
        if out.timed_out {
            break;
        }
    }
    Ok(())
}

fn walk_fragment(
    tree: &Tree,
    fragment: &Fragment,
    context: &WalkInput<'_>,
    initial_caller: Caller,
    jobs: &mut Vec<FragmentJob>,
    out: &mut MacroCalls,
) -> Result<(), PositionOverflow> {
    let first_job = jobs.len();
    let mut pending: Vec<(Node<'_>, u32, Caller)> = vec![(tree.root_node(), 0, initial_caller)];
    while let Some((node, depth, caller)) = pending.pop() {
        if deadline_spent(context.deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.capped = true;
            continue;
        }
        let kind = node.kind();
        let in_frame = within_synthetic_frame(node, fragment);
        if in_frame && matches!(kind, "call_expression" | "macro_invocation") {
            match emit_call_row(node, context, fragment, &caller) {
                Ok(Some(row)) => out.rows.push(row),
                Ok(None) => {}
                Err(NodeError::Text(error)) => {
                    tracing::warn!(%error, "macro argument call skipped; extraction is partial");
                    out.capped = true;
                }
                Err(NodeError::Position(error)) => return Err(error),
            }
        }
        let below = if in_frame {
            caller_below(node, kind, &fragment.wrapped, &caller)
        } else {
            caller.clone()
        };
        let Some(child_depth) = depth.checked_add(1) else {
            out.capped = true;
            continue;
        };
        crate::evidence::push_children(&mut pending, node, |child| {
            (child, child_depth, below.clone())
        });
        if in_frame && let Some(nested) = nested_macro_range(node, fragment) {
            match fragment.macro_depth.checked_add(1) {
                Some(nested_depth) => jobs.push(FragmentJob {
                    range: nested,
                    macro_depth: nested_depth,
                    caller,
                }),
                None => out.capped = true,
            }
        }
    }
    crate::evidence::reverse_from(jobs, first_job);
    Ok(())
}

fn within_synthetic_frame(node: Node<'_>, fragment: &Fragment) -> bool {
    node.start_byte() >= fragment.prefix_len
        && fragment
            .wrapped
            .len()
            .checked_sub(fragment.suffix_len)
            .is_some_and(|body_end| node.end_byte() <= body_end)
}

fn absolute_offset(fragment: &Fragment, synthetic_offset: usize) -> Option<usize> {
    synthetic_offset
        .checked_sub(fragment.prefix_len)?
        .checked_add(fragment.start)
}

fn parse_best_frame(
    body: &str,
    range: (usize, usize),
    macro_depth: u32,
    context: &WalkInput<'_>,
    parser: &mut Parser,
    out: &mut MacroCalls,
) -> Option<(Tree, Fragment)> {
    let mut best: Option<(usize, Tree, Fragment)> = None;
    for spec in FRAMES {
        if deadline_spent(context.deadline) {
            out.timed_out = true;
            break;
        }
        let fragment = Fragment::new(spec, body, range, macro_depth);
        let tree = match parse_within(parser, &fragment.wrapped, || {
            deadline_spent(context.deadline)
        }) {
            Parsed::Tree(tree) => tree,
            Parsed::Cancelled => {
                out.timed_out = true;
                break;
            }
            Parsed::NoTree => {
                tracing::warn!(
                    language = "rust",
                    "evidence walker parse produced no tree; extraction is partial"
                );
                out.capped = true;
                break;
            }
        };
        if !tree.root_node().has_error() {
            return Some((tree, fragment));
        }
        let calls = count_frame_calls(tree.root_node(), &fragment, context.deadline, out);
        if out.timed_out {
            break;
        }
        if best
            .as_ref()
            .is_none_or(|(best_calls, _, _)| calls > *best_calls)
        {
            best = Some((calls, tree, fragment));
        }
    }
    if out.timed_out {
        return None;
    }
    best.map(|(_, tree, fragment)| (tree, fragment))
}

fn count_frame_calls(
    root: Node<'_>,
    fragment: &Fragment,
    deadline: Option<&crate::util::Deadline>,
    out: &mut MacroCalls,
) -> usize {
    let mut count = 0usize;
    let mut pending = vec![(root, 0u32)];
    while let Some((node, depth)) = pending.pop() {
        if deadline_spent(deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            continue;
        }
        if within_synthetic_frame(node, fragment)
            && matches!(node.kind(), "call_expression" | "macro_invocation")
        {
            // KEEP saturating: call-node comparison count cannot exceed node count in bounded source
            count = count.saturating_add(1);
        }
        let Some(child_depth) = depth.checked_add(1) else {
            continue;
        };
        crate::evidence::push_children(&mut pending, node, |child| (child, child_depth));
    }
    count
}

fn token_tree_bounds(tokens: Node<'_>, source: &str) -> Result<(usize, usize), PositionOverflow> {
    let raw_start = tokens.start_byte();
    let raw_end = tokens.end_byte();
    let Some(slice) = source.as_bytes().get(raw_start..raw_end) else {
        return Ok((raw_start, raw_end));
    };
    let start = if matches!(slice.first(), Some(b'(' | b'[' | b'{')) {
        raw_start
            .checked_add(1)
            .ok_or(PositionOverflow::Arithmetic {
                operation: "the start of a macro token tree body",
            })?
    } else {
        raw_start
    };
    let end = if matches!(slice.last(), Some(b')' | b']' | b'}')) {
        raw_end.checked_sub(1).ok_or(PositionOverflow::Arithmetic {
            operation: "the end of a macro token tree body",
        })?
    } else {
        raw_end
    };
    Ok((start, end))
}

fn nested_macro_range(node: Node<'_>, fragment: &Fragment) -> Option<(usize, usize)> {
    if node.kind() != "macro_invocation" {
        return None;
    }
    let tokens = named_children(node).find(|n| n.kind() == "token_tree")?;
    let Ok((inner_start, inner_end)) = token_tree_bounds(tokens, &fragment.wrapped) else {
        return None;
    };
    let start = absolute_offset(fragment, inner_start)?;
    let end = absolute_offset(fragment, inner_end)?;
    (end <= fragment.end && start < end).then_some((start, end))
}

fn emit_call_row(
    node: Node<'_>,
    input: &WalkInput<'_>,
    fragment: &Fragment,
    enclosing: &Caller,
) -> Result<Option<(usize, usize, EvidenceItem)>, NodeError> {
    let wrapped = fragment.wrapped.as_str();
    let macro_call = node.kind() == "macro_invocation";
    let (Some(start), Some(end)) = (
        absolute_offset(fragment, node.start_byte()),
        absolute_offset(fragment, node.end_byte()),
    ) else {
        return Ok(None);
    };
    if end > fragment.end {
        return Ok(None);
    }
    let Some(callee) = callee_node(node, macro_call) else {
        return Ok(None);
    };
    let callee_text = node_text(wrapped, callee.start_byte(), callee.end_byte())?;
    let name = match simple_name(callee, wrapped)? {
        Some(name) => name,
        None => callee_text.trim_end_matches('!').to_owned(),
    };
    let call_span = crate::evidence::checked_span(
        (start, end),
        input.lines.position(start)?,
        input.lines.position(end)?,
        "macro argument call",
    );
    let site = CallSite {
        language: "rust",
        origin: EvidenceOrigin::RustCallWalker,
        qualified: qualified_callee(callee, wrapped)?,
        callee_text: callee_text.to_owned(),
        callee_name: name,
        caller: caller_text(enclosing)?,
        macro_call,
        span: call_span,
    };
    Ok(Some((start, end, call_item(String::new(), site))))
}
