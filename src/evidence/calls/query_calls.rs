// slugaudit-line-exception: approved-by=split-judge; reason=embedded tree-sitter query call extraction and linear preorder caller span index
//! Call-site evidence for the languages SlugAudit reads through an embedded
//! tree-sitter call query (the table is `call_queries.rs`), executed over the
//! already-parsed `tree_sitter::Tree`.

use super::call_queries::{CALL_QUERIES, CallQuery, MAX_CALLEE_DEPTH};
use super::site::{CallSite, call_item};
use crate::evidence::deadline_spent as spent;
use crate::evidence::{Extraction, NoTree, NodeError, TextRangeError, node_text};
use crate::model::{EvidenceOrigin, LineIndex, PositionOverflow};
use crate::util::Deadline;
use std::ops::ControlFlow;
use tree_sitter::{
    Node, Query, QueryCursor, QueryCursorOptions, QueryCursorState, StreamingIterator, Tree,
};

/// The deepest node a query match may start at: one below the walk depth cap.
const MAX_START_DEPTH: u32 = crate::evidence::MAX_WALK_DEPTH - 1;

/// Extracts call evidence for `language` when it has an embedded call query
/// (an empty, complete extraction otherwise), reusing the already-parsed `tree`.
pub(super) fn extract(
    language: &str,
    tree: &Tree,
    source: &str,
    lines: &LineIndex<'_>,
    deadline: Option<&Deadline>,
) -> Result<Extraction, PositionOverflow> {
    let Some(entry) = CALL_QUERIES.iter().find(|entry| entry.language == language) else {
        return Ok(Extraction::default());
    };
    if spent(deadline) {
        return Ok(Extraction::without_tree(NoTree::TimedOut));
    }
    let grammar = tree.language();
    let query = match entry.compiled(&grammar) {
        Ok(query) => query,
        Err(_logged_at_compile) => return Ok(Extraction::without_tree(NoTree::Failed)),
    };
    run(entry, query, tree, source, lines, deadline)
}

fn run(
    entry: &CallQuery,
    query: &Query,
    tree: &Tree,
    source: &str,
    lines: &LineIndex<'_>,
    deadline: Option<&Deadline>,
) -> Result<Extraction, PositionOverflow> {
    let mut out = Extraction::default();
    let functions = collect_functions(entry, tree, source, deadline, &mut out);
    if out.timed_out {
        return Ok(out);
    }
    let mut cut = false;
    let mut progress = |_state: &QueryCursorState| -> ControlFlow<()> {
        if spent(deadline) {
            cut = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut cursor = QueryCursor::new();
    cursor.set_max_start_depth(Some(MAX_START_DEPTH));
    let options = QueryCursorOptions::new().progress_callback(&mut progress);
    let mut matches =
        cursor.matches_with_options(query, tree.root_node(), source.as_bytes(), options);
    while let Some(found) = matches.next() {
        if spent(deadline) {
            out.timed_out = true;
            break;
        }
        for capture in found.captures {
            match call_site(entry, capture.node, source, lines, &functions) {
                Ok(Some(site)) => {
                    let key = format!("call:{}", out.items.len());
                    out.items.push(call_item(key, site));
                }
                Ok(None) => {
                    tracing::warn!(
                        language = entry.language,
                        "call capture has no call node above it; extraction is partial"
                    );
                    out.truncated = true;
                }
                Err(NodeError::Text(error)) => {
                    tracing::warn!(%error, "call capture skipped; extraction is partial");
                    out.truncated = true;
                }
                Err(NodeError::Position(error)) => return Err(error),
            }
        }
    }
    drop(matches);
    out.timed_out |= cut;
    Ok(out)
}

fn call_site(
    entry: &CallQuery,
    callee: Node<'_>,
    source: &str,
    lines: &LineIndex<'_>,
    functions: &FunctionIndex,
) -> Result<Option<CallSite>, NodeError> {
    let mut above = callee.parent();
    let mut call = None;
    for _ in 0..MAX_CALLEE_DEPTH {
        let Some(node) = above else { break };
        if node.kind() == entry.call_kind {
            call = Some(node);
            break;
        }
        above = node.parent();
    }
    let Some(call) = call else {
        return Ok(None);
    };
    let callee_text = node_text(source, call.start_byte(), callee.end_byte())?.to_owned();
    let callee_name = node_text(source, callee.start_byte(), callee.end_byte())?.to_owned();
    Ok(Some(CallSite {
        language: entry.language,
        origin: EvidenceOrigin::RawTree,
        qualified: super::site::is_qualified(&callee_text),
        callee_text,
        callee_name,
        caller: functions.enclosing_caller(call.start_byte(), call.end_byte()),
        macro_call: false,
        span: crate::evidence::range_span(
            (call.start_byte(), call.end_byte()),
            (call.start_position().row, call.end_position().row),
            lines,
            "call query",
        )?,
    }))
}

struct FunctionSpan {
    start: usize,
    end: usize,
    name: Option<String>,
    parent: Option<usize>,
}

struct FunctionIndex {
    spans: Vec<FunctionSpan>,
}

impl FunctionIndex {
    fn enclosing_caller(&self, call_start: usize, call_end: usize) -> Option<String> {
        let pos = self.spans.partition_point(|span| span.start <= call_start);
        let mut idx = pos.checked_sub(1)?;
        loop {
            let span = self.spans.get(idx)?;
            if span.end >= call_end {
                return span.name.clone();
            }
            idx = span.parent?;
        }
    }
}

fn collect_functions(
    entry: &CallQuery,
    tree: &Tree,
    source: &str,
    deadline: Option<&Deadline>,
    out: &mut Extraction,
) -> FunctionIndex {
    let mut spans: Vec<FunctionSpan> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut cursor = tree.walk();
    let mut depth: u32 = 0;

    loop {
        if spent(deadline) {
            out.timed_out = true;
            break;
        }
        let node = cursor.node();
        if entry.function_kinds.contains(&node.kind()) {
            let start = node.start_byte();
            let end = node.end_byte();
            while active
                .last()
                .and_then(|&idx| spans.get(idx))
                .is_some_and(|span| span.end <= start)
            {
                active.pop();
            }
            let name = match function_name(node, source) {
                Ok(name) => name,
                Err(error) => {
                    tracing::warn!(%error, "function name skipped; extraction is partial");
                    out.truncated = true;
                    None
                }
            };
            let idx = spans.len();
            spans.push(FunctionSpan {
                start,
                end,
                name,
                parent: active.last().copied(),
            });
            active.push(idx);
        }

        if depth < crate::evidence::MAX_WALK_DEPTH
            && let Some(next_depth) = depth.checked_add(1)
            && cursor.goto_first_child()
        {
            depth = next_depth;
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return FunctionIndex { spans };
            }
            if let Some(prev_depth) = depth.checked_sub(1) {
                depth = prev_depth;
            }
        }
    }

    FunctionIndex { spans }
}

fn function_name(function: Node<'_>, source: &str) -> Result<Option<String>, TextRangeError> {
    let mut current = function;
    let name = loop {
        if let Some(name) = current.child_by_field_name("name") {
            break Some(name);
        }
        match current.child_by_field_name("declarator") {
            Some(declarator) if declarator.kind() == "identifier" => break Some(declarator),
            Some(declarator) => current = declarator,
            None => break None,
        }
    };
    name.map(|name| node_text(source, name.start_byte(), name.end_byte()).map(str::to_owned))
        .transpose()
}
