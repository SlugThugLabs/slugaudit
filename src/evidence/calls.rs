// slugaudit-line-exception: approved-by=slugthuglabs; reason=the call dispatch and the Rust call walker it owns: the walk, its row emission and the Rust callee naming the macro re-parse shares
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
//! single-capture query would drop.

use crate::model::{EvidenceOrigin, PositionOverflow};
use std::collections::HashSet;
use std::rc::Rc;

mod call_queries;
mod macro_calls;
mod query_calls;
mod site;
use super::{Extraction, TextRangeError, WalkInput, named_children, node_text};
use site::{CallSite, call_item, is_qualified};
use tree_sitter_language_pack::{Node, Tree};

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
/// the Rust walker over `walker_tree` (the tree the binding walker read) for
/// Rust, the embedded call query for a language that has one (on a parse of
/// its own, within what is left of the deadline), and nothing (complete,
/// empty) for any other language. A position past `u32::MAX` is an error
/// that fails the file's extraction.
pub(super) fn extract(
    language: &str,
    walker_tree: &Tree,
    input: &WalkInput<'_>,
) -> Result<Extraction, PositionOverflow> {
    if language == "rust" {
        extract_rust(walker_tree, input)
    } else {
        query_calls::extract(language, input.source, input.lines, input.deadline)
    }
}

/// Extracts Rust call-site evidence from `tree`, the parse of one already
/// indexed source file. The result is truncated when a walk bound (tree
/// depth or macro nesting) was hit, a macro body could not be re-parsed, or
/// a node range did not slice the source, which the caller must report as
/// partial; it is timed out when the walk or a macro body's re-parse hit the
/// deadline.
fn extract_rust(tree: &Tree, input: &WalkInput<'_>) -> Result<Extraction, PositionOverflow> {
    let mut out = Extraction::default();
    let mut counter = 0usize;
    let mut seen = HashSet::new();
    walk(tree.root_node(), input, &mut out, &mut counter, &mut seen)?;
    Ok(out)
}

/// The caller every node in a subtree reports: the name of the nearest
/// enclosing [`RUST_FUNCTION_KINDS`] node (`None` at file scope, or when that
/// node has no `identifier` child, as for a closure or an `impl`), or the
/// error slicing that name, which fails each call under it. The walkers carry
/// it down their own stacks, so no call climbs `parent()` (which tree-sitter
/// answers by descending from the root, quadratic on a wide node).
type Caller = Result<Option<Rc<str>>, TextRangeError>;

/// The caller `node`'s children report: `node`'s own name when it is a
/// function-like node, else `inherited`.
fn caller_below(node: &Node, kind: &str, source: &str, inherited: &Caller) -> Caller {
    if RUST_FUNCTION_KINDS.contains(&kind) {
        function_name(node, source).map(|name| name.map(Rc::from))
    } else {
        inherited.clone()
    }
}

/// The `caller` a call row carries, or the error slicing its function's
/// name, which skips the call.
fn caller_text(enclosing: &Caller) -> Result<Option<String>, TextRangeError> {
    match enclosing {
        Ok(name) => Ok(name.as_deref().map(str::to_owned)),
        Err(error) => Err(*error),
    }
}

/// The text-derived fields of one call node.
struct CallParts {
    callee_text: String,
    callee_name: String,
    caller: Option<String>,
    qualified: bool,
}

fn call_parts(
    node: &Node,
    source: &str,
    macro_call: bool,
    enclosing: &Caller,
) -> Result<CallParts, TextRangeError> {
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
    let simple = match callee {
        Some(child) => simple_name(child, source)?,
        None => None,
    };
    let callee_name = match simple {
        Some(name) => name,
        None => callee_text.trim_end_matches('!').to_owned(),
    };
    Ok(CallParts {
        callee_text,
        callee_name,
        caller: caller_text(enclosing)?,
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
/// dedup identical to a recursive pre-order walk's. Each entry carries its
/// [`Caller`]. When the deadline runs out the walk stops and sets
/// `out.timed_out`.
fn walk(
    root: Node,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
    seen: &mut HashSet<(usize, usize)>,
) -> Result<(), PositionOverflow> {
    let mut pending: Vec<(Node, u32, Caller)> = vec![(root, 0, Ok(None))];
    while let Some((node, depth, caller)) = pending.pop() {
        if super::deadline_spent(input.deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        let kind = node.kind();
        if RUST_CALL_KINDS.contains(&kind.as_str()) {
            let macro_call = kind == "macro_invocation";
            match call_parts(&node, input.source, macro_call, &caller) {
                Ok(parts) => emit_call(&node, parts, input, out, counter, seen)?,
                Err(error) => {
                    tracing::warn!(%error, "Rust call node skipped; extraction is partial");
                    out.truncated = true;
                }
            }
        }
        let below = caller_below(&node, &kind, input.source, &caller);
        crate::evidence::push_children(&mut pending, &node, |child| {
            (child, depth + 1, below.clone())
        });
    }
    Ok(())
}

/// Pushes the call row for `node` (and, for a macro, the calls recovered from
/// its token tree), recording in `out` whether the macro scan was truncated
/// or timed out.
fn emit_call(
    node: &Node,
    parts: CallParts,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
    seen: &mut HashSet<(usize, usize)>,
) -> Result<(), PositionOverflow> {
    let macro_call = node.kind() == "macro_invocation";
    let key = format!("call:{counter}");
    *counter += 1;
    if seen.insert((node.start_byte(), node.end_byte())) {
        let site = CallSite {
            language: "rust",
            origin: EvidenceOrigin::RustCallWalker,
            callee_text: parts.callee_text,
            callee_name: parts.callee_name,
            caller: parts.caller.clone(),
            macro_call,
            qualified: parts.qualified,
            span: super::node_span(node, input.lines, "Rust call")?,
        };
        out.items.push(call_item(key, site));
    }
    if !macro_call {
        return Ok(());
    }
    let recovered = macro_calls::extract(node, input, parts.caller.as_deref())?;
    for (start, end, mut item) in recovered.rows {
        if seen.insert((start, end)) {
            item.key = format!("call:{counter}");
            *counter += 1;
            out.items.push(item);
        }
    }
    out.truncated |= recovered.capped;
    out.timed_out |= recovered.timed_out;
    Ok(())
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

/// The text of a function-like node's first `identifier` child, or `None`
/// when it has none.
fn function_name(node: &Node, source: &str) -> Result<Option<String>, TextRangeError> {
    match named_children(node).find(|child| child.kind() == "identifier") {
        Some(child) => Ok(Some(
            node_text(source, child.start_byte(), child.end_byte())?.to_owned(),
        )),
        None => Ok(None),
    }
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
