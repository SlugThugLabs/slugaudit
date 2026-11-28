// slugaudit-line-exception: approved-by=split-judge; reason=call dispatch and iterative Rust CST call walker sharing callee naming with macro re-parsing
//! Call-site evidence: the Rust walker, the query-driven languages
//! (`calls/query_calls.rs`), and the one row builder both feed.

use crate::model::{EvidenceOrigin, PositionOverflow};
use std::collections::HashSet;
use std::rc::Rc;

mod call_queries;
mod macro_calls;
mod query_calls;
mod site;
use super::{Extraction, TextRangeError, WalkInput, named_children, node_text};
use site::{CallSite, call_item, is_qualified};
use tree_sitter::{Language, Node, Parser, Tree};

const RUST_CALL_KINDS: &[&str] = &["call_expression", "macro_invocation"];
const RUST_FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_signature_item",
    "closure_expression",
    "impl_item",
    "trait_item",
];
const RUST_SCOPE_RESET_KINDS: &[&str] = &[
    "mod_item",
    "static_item",
    "const_item",
    "struct_item",
    "enum_item",
    "union_item",
    "type_item",
];

/// Every language call evidence is extracted for: Rust (the walker) and each
/// language with an embedded call query.
pub(crate) fn call_languages() -> impl Iterator<Item = &'static str> {
    std::iter::once("rust").chain(
        call_queries::CALL_QUERIES
            .iter()
            .map(|entry| entry.language),
    )
}

/// Extracts call-site evidence for one already parsed file in `language` from
/// `walker_tree` without re-parsing the outer file.
pub(super) fn extract(
    language: &str,
    walker_tree: &Tree,
    input: &WalkInput<'_>,
) -> Result<Extraction, PositionOverflow> {
    if language == "rust" {
        extract_rust(walker_tree, input)
    } else {
        query_calls::extract(
            language,
            walker_tree,
            input.source,
            input.lines,
            input.deadline,
        )
    }
}

struct RustWalkState {
    out: Extraction,
    counter: usize,
    seen: HashSet<(usize, usize)>,
}

fn extract_rust(tree: &Tree, input: &WalkInput<'_>) -> Result<Extraction, PositionOverflow> {
    let grammar = tree.language();
    let mut macro_parser: Option<Parser> = None;
    let mut state = RustWalkState {
        out: Extraction::default(),
        counter: 0,
        seen: HashSet::new(),
    };
    walk(
        tree.root_node(),
        input,
        &grammar,
        &mut macro_parser,
        &mut state,
    )?;
    Ok(state.out)
}

type Caller = Result<Option<Rc<str>>, TextRangeError>;

fn caller_below(node: Node<'_>, kind: &str, source: &str, inherited: &Caller) -> Caller {
    if RUST_FUNCTION_KINDS.contains(&kind) {
        function_name(node, source).map(|name| name.map(Rc::from))
    } else if RUST_SCOPE_RESET_KINDS.contains(&kind) {
        Ok(None)
    } else {
        inherited.clone()
    }
}

fn caller_text(enclosing: &Caller) -> Result<Option<String>, TextRangeError> {
    match enclosing {
        Ok(name) => Ok(name.as_deref().map(str::to_owned)),
        Err(error) => Err(*error),
    }
}

struct CallParts {
    callee_text: String,
    callee_name: String,
    caller: Option<String>,
    qualified: bool,
}

fn call_parts(
    node: Node<'_>,
    source: &str,
    macro_call: bool,
    enclosing: &Caller,
) -> Result<CallParts, TextRangeError> {
    let callee = callee_node(node, macro_call);
    let callee_text = match callee {
        Some(child) => node_text(source, child.start_byte(), child.end_byte())?,
        None => node_text(source, node.start_byte(), node.end_byte())?,
    }
    .to_owned();
    let qualified = match callee {
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

fn qualified_callee(callee: Node<'_>, source: &str) -> Result<bool, TextRangeError> {
    let path = match generic_function_inner(callee) {
        Some(inner) => inner,
        None => callee,
    };
    match path.kind() {
        "field_expression" | "identifier" => Ok(false),
        "scoped_identifier" => Ok(true),
        _ => Ok(is_qualified(node_text(
            source,
            path.start_byte(),
            path.end_byte(),
        )?)),
    }
}

fn generic_function_inner(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "generic_function" {
        node.child_by_field_name("function")
    } else {
        None
    }
}

fn walk<'tree>(
    root: Node<'tree>,
    input: &WalkInput<'_>,
    grammar: &Language,
    macro_parser: &mut Option<Parser>,
    state: &mut RustWalkState,
) -> Result<(), PositionOverflow> {
    let mut pending: Vec<(Node<'tree>, u32, Caller)> = vec![(root, 0, Ok(None))];
    while let Some((node, depth, caller)) = pending.pop() {
        if super::deadline_spent(input.deadline) {
            state.out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            state.out.truncated = true;
            continue;
        }
        let kind = node.kind();
        if RUST_CALL_KINDS.contains(&kind) {
            let macro_call = kind == "macro_invocation";
            match call_parts(node, input.source, macro_call, &caller) {
                Ok(parts) => emit_call(node, parts, input, grammar, macro_parser, state)?,
                Err(error) => {
                    tracing::warn!(%error, "Rust call node skipped; extraction is partial");
                    state.out.truncated = true;
                }
            }
        }
        let below = caller_below(node, kind, input.source, &caller);
        let Some(child_depth) = depth.checked_add(1) else {
            state.out.truncated = true;
            continue;
        };
        crate::evidence::push_children(&mut pending, node, |child| {
            (child, child_depth, below.clone())
        });
    }
    Ok(())
}

fn emit_call(
    node: Node<'_>,
    parts: CallParts,
    input: &WalkInput<'_>,
    grammar: &Language,
    macro_parser: &mut Option<Parser>,
    state: &mut RustWalkState,
) -> Result<(), PositionOverflow> {
    let macro_call = node.kind() == "macro_invocation";
    let key = format!("call:{}", state.counter);
    let Some(next) = state.counter.checked_add(1) else {
        tracing::warn!("call key counter exhausted; extraction is partial");
        state.out.truncated = true;
        return Ok(());
    };
    state.counter = next;
    if state.seen.insert((node.start_byte(), node.end_byte())) {
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
        state.out.items.push(call_item(key, site));
    }
    if !macro_call {
        return Ok(());
    }
    let recovered =
        macro_calls::extract(node, input, parts.caller.as_deref(), grammar, macro_parser)?;
    for (start, end, mut item) in recovered.rows {
        if state.seen.insert((start, end)) {
            let Some(next) = state.counter.checked_add(1) else {
                tracing::warn!("call key counter exhausted; extraction is partial");
                state.out.truncated = true;
                break;
            };
            item.key = format!("call:{}", state.counter);
            state.counter = next;
            state.out.items.push(item);
        }
    }
    state.out.truncated |= recovered.capped;
    state.out.timed_out |= recovered.timed_out;
    Ok(())
}

fn callee_node(node: Node<'_>, macro_call: bool) -> Option<Node<'_>> {
    if macro_call {
        node.child_by_field_name("macro").or_else(|| {
            named_children(node).find(|child| {
                !super::rust_doc::is_rust_comment_kind(child.kind()) && child.kind() != "token_tree"
            })
        })
    } else {
        node.child_by_field_name("function")
    }
}

fn simple_name(node: Node<'_>, source: &str) -> Result<Option<String>, TextRangeError> {
    let mut current = node;
    loop {
        if current.kind() == "identifier" || current.kind() == "field_identifier" {
            return Ok(Some(
                node_text(source, current.start_byte(), current.end_byte())?.to_owned(),
            ));
        }
        if let Some(inner) = generic_function_inner(current) {
            current = inner;
            continue;
        }
        let next = current
            .child_by_field_name("name")
            .or_else(|| current.child_by_field_name("field"))
            .or_else(|| {
                named_children(current)
                    .filter(|child| !super::rust_doc::is_rust_comment_kind(child.kind()))
                    .last()
            });
        let Some(child) = next else {
            return Ok(None);
        };
        current = child;
    }
}

fn function_name(node: Node<'_>, source: &str) -> Result<Option<String>, TextRangeError> {
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
