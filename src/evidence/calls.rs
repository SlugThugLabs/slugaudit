// slugaudit-line-exception: approved-by=slugthug; reason=the call dispatch and the Rust call walker it owns: the walk, its row emission and the Rust callee naming the macro re-parse shares
//! Call-site evidence: the Rust walker, the query-driven languages
//! (`calls/query_calls.rs`), and the one row builder both feed.
//!
//! This module deliberately reports syntax facts, not a compiler-grade call
//! graph. A Tree-sitter call node proves that source contains a call-shaped
//! expression and gives us its span; name resolution can still be ambiguous
//! for traits, generics, function pointers, macros, and dynamic dispatch.
//!
//! Rust keeps a walker rather than an embedded query: it re-parses macro
//! token trees (`calls/macro_calls.rs`), which a query cannot reach, and it
//! records every call node — including a callee no pattern names, such as
//! `(f)()` or `f()()`, whose whole call text becomes the callee — which a
//! single-capture query would drop, changing existing counts.

use crate::model::{EvidenceOrigin, LineIndex};
use std::collections::HashSet;

mod call_queries;
mod macro_calls;
mod query_calls;
mod site;
use super::{Extraction, TextRangeError, node_text};
use crate::util::Deadline;
use site::{CallSite, call_item, is_qualified};
use tree_sitter_language_pack::Node;

const RUST_CALL_KINDS: &[&str] = &["call_expression", "macro_invocation"];
const RUST_FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_signature_item",
    "closure_expression",
    "impl_item",
    "trait_item",
];

/// Every language call evidence is extracted for: Rust (the walker) and each
/// language with an embedded call query. Derived from the query table, so a
/// `covers` list built from it cannot drift from what is extracted.
pub(crate) fn call_languages() -> impl Iterator<Item = &'static str> {
    std::iter::once("rust").chain(
        call_queries::CALL_QUERIES
            .iter()
            .map(|entry| entry.language),
    )
}

/// Extracts call-site evidence for one already indexed file in `language`:
/// the Rust walker for Rust, the embedded call query for a language that has
/// one, and nothing (complete, empty) for any other language. Every parse
/// runs within what is left of `deadline`.
pub(crate) fn extract(
    language: &str,
    source: &str,
    lines: &LineIndex<'_>,
    deadline: Option<&Deadline>,
) -> Extraction {
    if language == "rust" {
        extract_rust(source, lines, deadline)
    } else {
        query_calls::extract(language, source, lines, deadline)
    }
}

/// What stays fixed across one file's call walk.
struct Input<'a> {
    source: &'a str,
    lines: &'a LineIndex<'a>,
    /// Bounds the file's parse and every macro-body re-parse (one budget).
    deadline: Option<&'a Deadline>,
}

/// Extracts Rust call-site evidence from one already indexed source file.
/// The result is truncated when a walk bound (tree depth or macro nesting)
/// was hit, the parser could not produce a tree, or a node range did not
/// slice the source, which the caller must report as partial; it is timed
/// out when the file's parse or a macro body's re-parse hit `deadline`.
fn extract_rust(source: &str, lines: &LineIndex<'_>, deadline: Option<&Deadline>) -> Extraction {
    let tree = match super::parse_tree("rust", source, deadline) {
        Ok(tree) => tree,
        Err(why) => return Extraction::without_tree(why),
    };
    let input = Input {
        source,
        lines,
        deadline,
    };
    let mut out = Extraction::default();
    let mut counter = 0usize;
    let mut seen = HashSet::new();
    walk(tree.root_node(), &input, &mut out, &mut counter, &mut seen);
    out
}

/// The text-derived fields of one call node.
struct CallParts {
    callee_text: String,
    callee_name: String,
    caller: Option<String>,
    qualified: bool,
}

fn call_parts(node: &Node, source: &str, macro_call: bool) -> Result<CallParts, TextRangeError> {
    let callee = callee_node(node, macro_call);
    let callee_text = match &callee {
        Some(child) => node_text(source, child.start_byte(), child.end_byte())?,
        None => node_text(source, node.start_byte(), node.end_byte())?,
    }
    .to_owned();
    let qualified = match &callee {
        Some(child) => qualified_callee(child, source)?,
        None => is_qualified(&callee_text),
    };
    let callee_name = match callee {
        Some(child) => simple_name(child, source)?,
        None => None,
    }
    .unwrap_or_else(|| callee_text.trim_end_matches('!').to_owned());
    Ok(CallParts {
        callee_text,
        callee_name,
        caller: enclosing_caller(node, source)?,
        qualified,
    })
}

/// Whether a Rust callee is a `::` path. A turbofish (`foo::<u8>`,
/// `x.parse::<u32>`) is a `generic_function`, whose `::` introduces type
/// arguments, not a path: only its `function` part is checked.
fn qualified_callee(callee: &Node, source: &str) -> Result<bool, TextRangeError> {
    let path = match generic_function_inner(callee) {
        Some(inner) => inner,
        None => callee.clone(),
    };
    Ok(is_qualified(node_text(
        source,
        path.start_byte(),
        path.end_byte(),
    )?))
}

/// The `function` part of a turbofish callee (`generic_function`), or `None`
/// for any other node.
fn generic_function_inner(node: &Node) -> Option<Node> {
    if node.kind() == "generic_function" {
        node.child_by_field_name("function")
    } else {
        None
    }
}

/// Pre-order walk with an explicit heap stack, so the thread stack stays flat
/// however deep the file nests (see [`crate::evidence::MAX_WALK_DEPTH`]).
/// Children pop in source order, which keeps `call:N` keys and the first-wins
/// dedup identical to the recursive walk this replaced.
fn walk(
    root: Node,
    input: &Input<'_>,
    out: &mut Extraction,
    counter: &mut usize,
    seen: &mut HashSet<(usize, usize)>,
) {
    let mut pending = vec![(root, 0)];
    while let Some((node, depth)) = pending.pop() {
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        if RUST_CALL_KINDS.contains(&node.kind().as_str()) {
            let macro_call = node.kind() == "macro_invocation";
            match call_parts(&node, input.source, macro_call) {
                Ok(parts) => emit_call(&node, parts, input, out, counter, seen),
                Err(error) => {
                    tracing::warn!(%error, "Rust call node skipped; extraction is partial");
                    out.truncated = true;
                }
            }
        }
        crate::evidence::push_children(&mut pending, &node, |child| (child, depth + 1));
    }
}

/// Pushes the call row for `node` (and, for a macro, the calls recovered from
/// its token tree), recording in `out` whether the macro scan was truncated
/// or timed out.
fn emit_call(
    node: &Node,
    parts: CallParts,
    input: &Input<'_>,
    out: &mut Extraction,
    counter: &mut usize,
    seen: &mut HashSet<(usize, usize)>,
) {
    let macro_call = node.kind() == "macro_invocation";
    let key = format!("call:{counter}");
    *counter += 1;
    if seen.insert((node.start_byte(), node.end_byte())) {
        let site = CallSite {
            language: "rust",
            origin: EvidenceOrigin::RustCallWalker,
            callee_text: parts.callee_text,
            callee_name: parts.callee_name,
            caller: parts.caller,
            macro_call,
            qualified: parts.qualified,
            span: super::node_span(node, input.lines, "Rust call"),
        };
        out.items.push(call_item(key, site));
    }
    if !macro_call {
        return;
    }
    let recovered = macro_calls::extract(node, input);
    for (start, end, mut item) in recovered.rows {
        if seen.insert((start, end)) {
            item.key = format!("call:{counter}");
            *counter += 1;
            out.items.push(item);
        }
    }
    out.truncated |= recovered.capped;
    out.timed_out |= recovered.timed_out;
}

fn callee_node(node: &Node, macro_call: bool) -> Option<Node> {
    named_children(node).find(|child| {
        macro_call
            || child.kind() == "identifier"
            || child.kind() == "scoped_identifier"
            || child.kind() == "field_expression"
            || child.kind() == "generic_function"
    })
}

/// The simple name of a callee. `field_expression` (`v.unwrap`) recurses to
/// its last named child — the `field_identifier` — so a method call reports
/// `unwrap`, while `callee_text` keeps the receiver for context. A turbofish
/// (`generic_function`: `foo::<u8>`, `x.parse::<u32>`) recurses into its
/// `function` part, not its type arguments, so it reports `foo` / `parse`.
/// A loop, not recursion: the descent follows file content, so its length is
/// untrusted.
fn simple_name(node: Node, source: &str) -> Result<Option<String>, TextRangeError> {
    let mut current = node;
    loop {
        if current.kind() == "identifier" || current.kind() == "field_identifier" {
            return Ok(Some(
                node_text(source, current.start_byte(), current.end_byte())?.to_owned(),
            ));
        }
        if let Some(inner) = generic_function_inner(&current) {
            current = inner;
            continue;
        }
        match named_children(&current).last() {
            Some(child) => current = child,
            None => return Ok(None),
        }
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

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
