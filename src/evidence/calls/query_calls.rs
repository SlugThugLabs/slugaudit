//! Call-site evidence for the languages SlugAudit reads through an embedded
//! tree-sitter call query (the table is `call_queries.rs`).
//!
//! Each query has exactly one capture, `@callee`: the simple name of the
//! called function or method. The call node it belongs to, its span, the
//! callee text as written and the enclosing caller are derived here, once,
//! for every language; the row itself is built by the shared
//! [`super::site::call_item`]. Rust is not in this table: its walker also re-parses
//! macro bodies and keeps callees no query pattern names (see `calls.rs`).

use super::call_queries::{CALL_QUERIES, CallQuery, MAX_CALLEE_DEPTH};
use super::site::{CallSite, call_item};
use crate::evidence::{Extraction, NoTree, TextRangeError, node_text};
use crate::model::{EvidenceOrigin, LineIndex};
use crate::util::Deadline;
use std::ops::ControlFlow;
use tree_sitter::{
    Language, Node, ParseOptions, ParseState, Parser, Query, QueryCursor, QueryCursorOptions,
    QueryCursorState, StreamingIterator, Tree,
};

/// Extracts call evidence for `language` when it has an embedded call query
/// (an empty, complete extraction otherwise). The parse and the query cursor
/// both run within what is left of `deadline`.
pub(super) fn extract(
    language: &str,
    source: &str,
    lines: &LineIndex<'_>,
    deadline: Option<&Deadline>,
) -> Extraction {
    let Some(entry) = CALL_QUERIES.iter().find(|entry| entry.language == language) else {
        return Extraction::default();
    };
    let grammar = match tree_sitter_language_pack::get_language(entry.language) {
        Ok(grammar) => grammar,
        Err(error) => {
            tracing::warn!(language = entry.language, %error, "call query has no grammar; extraction is partial");
            return Extraction::without_tree(NoTree::Failed);
        }
    };
    let query = match entry.compiled(&grammar) {
        Ok(query) => query,
        // Logged once, when the query failed to compile; the file's call
        // evidence is reported partial.
        Err(_logged_at_compile) => return Extraction::without_tree(NoTree::Failed),
    };
    let tree = match parse_within(&grammar, source, deadline) {
        Ok(tree) => tree,
        Err(why) => return Extraction::without_tree(why),
    };
    run(entry, query, &tree, source, lines, deadline)
}

/// Whether `deadline` has less than a millisecond left: the same rule
/// [`crate::evidence::parse_tree`] applies.
fn spent(deadline: Option<&Deadline>) -> bool {
    deadline.is_some_and(|deadline| deadline.remaining_millis().is_none())
}

/// Parses `source` with a parser of its own (dropped after, so a cancelled
/// parse leaves no state behind), cancelled through tree-sitter's progress
/// callback once `deadline` is spent.
fn parse_within(
    grammar: &Language,
    source: &str,
    deadline: Option<&Deadline>,
) -> Result<Tree, NoTree> {
    if spent(deadline) {
        return Err(NoTree::TimedOut);
    }
    let mut parser = Parser::new();
    if let Err(error) = parser.set_language(grammar) {
        tracing::warn!(%error, "call query parser could not take its grammar; extraction is partial");
        return Err(NoTree::Failed);
    }
    let bytes = source.as_bytes();
    let mut read = |offset: usize, _| bytes.get(offset..).unwrap_or_default();
    let mut cancelled = false;
    let mut progress = |_state: &ParseState| -> ControlFlow<()> {
        if spent(deadline) {
            cancelled = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    match parser.parse_with_options(&mut read, None, Some(options)) {
        Some(tree) => Ok(tree),
        None if cancelled => Err(NoTree::TimedOut),
        None => {
            tracing::warn!("call query parse produced no tree; extraction is partial");
            Err(NoTree::Failed)
        }
    }
}

/// Runs `query` over `tree`, one row per match. The cursor stops at the
/// deadline (the rows so far are kept and the extraction is timed out), and
/// starts no match deeper than [`crate::evidence::MAX_WALK_DEPTH`] — the bound
/// every walker shares; the bindings walker reads the same tree and reports
/// such a file partial.
fn run(
    entry: &CallQuery,
    query: &Query,
    tree: &Tree,
    source: &str,
    lines: &LineIndex<'_>,
    deadline: Option<&Deadline>,
) -> Extraction {
    let mut out = Extraction::default();
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
    cursor.set_max_start_depth(Some(crate::evidence::MAX_WALK_DEPTH.saturating_sub(1)));
    let options = QueryCursorOptions::new().progress_callback(&mut progress);
    let mut matches =
        cursor.matches_with_options(query, tree.root_node(), source.as_bytes(), options);
    let mut counter = 0usize;
    while let Some(found) = matches.next() {
        if spent(deadline) {
            out.timed_out = true;
            break;
        }
        for capture in found.captures {
            match call_site(entry, capture.node, source, lines) {
                Ok(Some(site)) => {
                    out.items.push(call_item(format!("call:{counter}"), site));
                    counter += 1;
                }
                Ok(None) => {
                    tracing::warn!(
                        language = entry.language,
                        "call capture has no call node above it; extraction is partial"
                    );
                    out.truncated = true;
                }
                Err(error) => {
                    tracing::warn!(%error, "call capture skipped; extraction is partial");
                    out.truncated = true;
                }
            }
        }
    }
    drop(matches);
    out.timed_out |= cut;
    out
}

/// The call site of one `@callee` capture, or `None` when no call node of
/// `entry.call_kind` sits within [`MAX_CALLEE_DEPTH`] above it (an embedded
/// query that captures outside a call).
fn call_site(
    entry: &CallQuery,
    callee: Node<'_>,
    source: &str,
    lines: &LineIndex<'_>,
) -> Result<Option<CallSite>, TextRangeError> {
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
    // The callee as written: from the start of the call (the receiver, if
    // any) to the end of the captured name — `obj.method`, `pkg.F`, `f`.
    let callee_text = node_text(source, call.start_byte(), callee.end_byte())?.to_owned();
    let callee_name = node_text(source, callee.start_byte(), callee.end_byte())?.to_owned();
    Ok(Some(CallSite {
        language: entry.language,
        origin: EvidenceOrigin::RawTree,
        qualified: super::site::is_qualified(&callee_text),
        callee_text,
        callee_name,
        caller: enclosing_caller(entry, call, source)?,
        macro_call: false,
        span: crate::evidence::range_span(
            (call.start_byte(), call.end_byte()),
            (call.start_position().row, call.end_position().row),
            lines,
            "call query",
        ),
    }))
}

/// The name of the nearest enclosing function-like node, or `None` at file
/// scope or inside an anonymous function (as the Rust walker does for a
/// closure).
fn enclosing_caller(
    entry: &CallQuery,
    call: Node<'_>,
    source: &str,
) -> Result<Option<String>, TextRangeError> {
    let mut current = call.parent();
    while let Some(node) = current {
        if entry.function_kinds.contains(&node.kind()) {
            return function_name(node, source);
        }
        current = node.parent();
    }
    Ok(None)
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

#[cfg(test)]
#[path = "query_calls_tests.rs"]
mod tests;
