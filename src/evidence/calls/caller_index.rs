//! Pre-collected enclosing-function span index for query-based call extraction.
//!
//! Walks the CST once in `O(N)` preorder with a `TreeCursor` so each `@callee`
//! capture resolves its enclosing caller in `O(log F + depth)` instead of
//! calling tree-sitter's `O(width)` `Node::parent()` per capture.

use super::call_queries::CallQuery;
use crate::evidence::deadline_spent as spent;
use crate::evidence::{Extraction, TextRangeError, node_text};
use crate::util::Deadline;
use tree_sitter::{Node, Tree};

struct FunctionSpan {
    start: usize,
    end: usize,
    name: Option<String>,
    parent: Option<usize>,
}

/// Preorder index of function-like spans in one parsed file.
pub(super) struct FunctionIndex {
    spans: Vec<FunctionSpan>,
}

impl FunctionIndex {
    /// The name of the nearest enclosing function-like node, or `None` at file
    /// scope or inside an anonymous function (as the Rust walker does for a
    /// closure).
    pub(super) fn enclosing_caller(&self, call_start: usize, call_end: usize) -> Option<String> {
        let pos = self.spans.partition_point(|span| span.start <= call_start);
        let mut idx = pos.checked_sub(1)?;
        loop {
            let span = self.spans.get(idx)?;
            if span.end >= call_end {
                // KEEP clone: multiple calls inside one function share its caller name
                return span.name.clone();
            }
            idx = span.parent?;
        }
    }
}

/// Collects every function-like node span in a single `O(N)` preorder cursor
/// walk so caller lookup per `@callee` does not call `Node::parent()`.
pub(super) fn collect_functions(
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

/// A function node's `name` field, or — for C, whose name sits in a
/// declarator chain (`function_definition` → `function_declarator` →
/// `identifier`, possibly through `pointer_declarator`) — the identifier at
/// the end of its `declarator` fields. A loop: the chain follows file content.
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
