// slugaudit-line-exception: approved-by=split-judge; reason=iterative tree-sitter-rust CST walker expanding grouped use declarations under depth, segment, and target caps
//! Rust `use`-tree CST parser: parses a `use ...;` snippet with `tree-sitter-rust`
//! and iteratively expands nested brace trees (`use crate::{a, b::*, foo::{self, bar as x}};`)
//! into deduplicated import targets.

use std::cell::RefCell;
use std::collections::HashSet;
use tree_sitter::{Node, Parser, Tree};

/// Maximum brace nesting depth allowed when expanding a Rust `use` tree.
const MAX_USE_TREE_DEPTH: usize = 32;

/// Maximum `::`-separated path segments allowed in any expanded `use` path.
const MAX_PATH_SEGMENTS: usize = 64;

/// Maximum number of expanded targets allowed from a single `use` statement.
const MAX_EXPANDED_TARGETS: usize = 256;

thread_local! {
    static RUST_USE_PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
}

fn parse_rust_snippet(raw: &str) -> Option<Tree> {
    RUST_USE_PARSER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let grammar = match tree_sitter_language_pack::get_language("rust") {
                Ok(lang) => lang,
                Err(error) => {
                    tracing::warn!(%error, "failed to load bundled rust grammar for use-tree");
                    return None;
                }
            };
            let mut parser = Parser::new();
            if let Err(error) = parser.set_language(&grammar) {
                tracing::warn!(%error, "failed to configure rust parser for use-tree");
                return None;
            }
            *slot = Some(parser);
        }
        slot.as_mut()?.parse(raw, None)
    })
}

/// Parses a raw Rust `use` statement into expanded targets via `tree-sitter-rust`.
///
/// Returns `None` for non-`use` items (such as `extern crate`), malformed
/// `use` trees, or trees exceeding expansion bounds; `Some(vec![])` for valid
/// empty-brace `use crate::foo::{};`; and `Some(targets)` for valid `use`
/// trees.
pub(crate) fn parse_rust_import(raw: &str) -> Option<Vec<String>> {
    if raw.trim().is_empty() {
        return None;
    }
    let tree = parse_rust_snippet(raw)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let use_decl = single_use_declaration(root)?;
    let argument = use_decl.child_by_field_name("argument")?;
    expand_use_argument(argument, raw)
}

fn single_use_declaration(root: Node<'_>) -> Option<Node<'_>> {
    let mut found = None;
    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() {
                match child.kind() {
                    "line_comment" | "block_comment" => {}
                    "use_declaration" if found.is_none() => found = Some(child),
                    _ => return None,
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    let decl = found?;
    let mut decl_cursor = decl.walk();
    let mut saw_semi = false;
    if decl_cursor.goto_first_child() {
        loop {
            let child = decl_cursor.node();
            if !child.is_named() && child.kind() == ";" {
                saw_semi = true;
            }
            if !decl_cursor.goto_next_sibling() {
                break;
            }
        }
    }
    saw_semi.then_some(decl)
}

struct PendingTree<'tree> {
    node: Node<'tree>,
    prefix: String,
    segments: usize,
    brace_depth: usize,
}

#[derive(Default)]
struct ExpansionOut {
    targets: Vec<String>,
    seen: HashSet<String>,
}

impl ExpansionOut {
    fn push_unique(&mut self, specifier: String) -> Option<()> {
        if self.seen.contains(&specifier) {
            return Some(());
        }
        (self.targets.len() < MAX_EXPANDED_TARGETS).then_some(())?;
        self.seen.insert(specifier.clone());
        self.targets.push(specifier);
        Some(())
    }
}

fn expand_use_argument(argument: Node<'_>, source: &str) -> Option<Vec<String>> {
    let mut out = ExpansionOut::default();
    let mut stack = vec![PendingTree {
        node: argument,
        prefix: String::new(),
        segments: 0,
        brace_depth: 0,
    }];

    while let Some(mut item) = stack.pop() {
        let mut node = item.node;
        if node.kind() == "use_as_clause" {
            node = node.child_by_field_name("path")?;
        }
        match node.kind() {
            "identifier" | "crate" | "super" => {
                let seg = slice_ident(node, source)?;
                append_segment(&mut item.prefix, &mut item.segments, seg)?;
                out.push_unique(item.prefix)?;
            }
            "self" => {
                if item.segments == 0 {
                    if item.prefix == "::" {
                        return None;
                    }
                    append_segment(&mut item.prefix, &mut item.segments, "self")?;
                } else {
                    item.segments = item
                        .segments
                        .checked_add(1)
                        .filter(|&n| n <= MAX_PATH_SEGMENTS)?;
                }
                out.push_unique(item.prefix)?;
            }
            "scoped_identifier" => {
                let (leading_colon, segs) = collect_scoped_segments(node, source)?;
                apply_leading_and_segments(
                    &mut item.prefix,
                    &mut item.segments,
                    leading_colon,
                    &segs,
                )?;
                out.push_unique(item.prefix)?;
            }
            "use_wildcard" => {
                expand_wildcard(node, source, &mut item.prefix, &mut item.segments)?;
                out.push_unique(item.prefix)?;
            }
            "scoped_use_list" => {
                let next_depth = item
                    .brace_depth
                    .checked_add(1)
                    .filter(|&d| d <= MAX_USE_TREE_DEPTH)?;
                if let Some(path_node) = node.child_by_field_name("path") {
                    let (leading_colon, segs) = collect_scoped_segments(path_node, source)?;
                    apply_leading_and_segments(
                        &mut item.prefix,
                        &mut item.segments,
                        leading_colon,
                        &segs,
                    )?;
                } else {
                    if !item.prefix.is_empty() {
                        return None;
                    }
                    item.prefix.push_str("::");
                }
                let list = node.child_by_field_name("list")?;
                push_use_list_children(&mut stack, list, &item.prefix, item.segments, next_depth)?;
            }
            "use_list" => {
                let next_depth = item
                    .brace_depth
                    .checked_add(1)
                    .filter(|&d| d <= MAX_USE_TREE_DEPTH)?;
                push_use_list_children(&mut stack, node, &item.prefix, item.segments, next_depth)?;
            }
            _ => return None,
        }
    }
    Some(out.targets)
}

fn push_use_list_children<'tree>(
    stack: &mut Vec<PendingTree<'tree>>,
    list: Node<'tree>,
    prefix: &str,
    segments: usize,
    brace_depth: usize,
) -> Option<()> {
    if list.kind() != "use_list" {
        return None;
    }
    let mut children = Vec::new();
    let mut cursor = list.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() && !matches!(child.kind(), "line_comment" | "block_comment") {
                children.push(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    for child in children.into_iter().rev() {
        stack.push(PendingTree {
            node: child,
            prefix: prefix.to_owned(),
            segments,
            brace_depth,
        });
    }
    Some(())
}

fn expand_wildcard(
    node: Node<'_>,
    source: &str,
    prefix: &mut String,
    segments: &mut usize,
) -> Option<()> {
    let mut inner_path = None;
    let mut has_double_colon = false;
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() {
                if !matches!(child.kind(), "line_comment" | "block_comment") {
                    if inner_path.is_some() {
                        return None;
                    }
                    inner_path = Some(child);
                }
            } else if child.kind() == "::" {
                has_double_colon = true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if let Some(path_node) = inner_path {
        let (leading_colon, segs) = collect_scoped_segments(path_node, source)?;
        apply_leading_and_segments(prefix, segments, leading_colon, &segs)?;
    } else if has_double_colon {
        return None;
    }
    (*segments > 0).then_some(())?;
    append_segment(prefix, segments, "*")
}

/// Iteratively unwraps a `scoped_identifier` or leaf segment node into
/// `(has_leading_double_colon, ordered_segments)`, aborting immediately if the
/// chain exceeds [`MAX_PATH_SEGMENTS`].
fn collect_scoped_segments<'a>(node: Node<'_>, source: &'a str) -> Option<(bool, Vec<&'a str>)> {
    let mut current = node;
    let mut reversed = Vec::new();
    while current.kind() == "scoped_identifier" {
        if reversed.len() >= MAX_PATH_SEGMENTS {
            return None;
        }
        let name = current.child_by_field_name("name")?;
        reversed.push(slice_ident(name, source)?);
        let Some(next) = current.child_by_field_name("path") else {
            reversed.reverse();
            return Some((true, reversed));
        };
        current = next;
    }
    if reversed.len() >= MAX_PATH_SEGMENTS {
        return None;
    }
    reversed.push(slice_ident(current, source)?);
    reversed.reverse();
    Some((false, reversed))
}

fn apply_leading_and_segments(
    prefix: &mut String,
    segments: &mut usize,
    leading_colon: bool,
    segs: &[&str],
) -> Option<()> {
    if leading_colon {
        if !prefix.is_empty() {
            return None;
        }
        prefix.push_str("::");
    }
    for &seg in segs {
        append_segment(prefix, segments, seg)?;
    }
    Some(())
}

fn append_segment(prefix: &mut String, segments: &mut usize, segment: &str) -> Option<()> {
    let needs_sep = *segments > 0;
    let next = segments
        .checked_add(1)
        .filter(|&n| n <= MAX_PATH_SEGMENTS)?;
    if needs_sep {
        prefix.push_str("::");
    }
    prefix.push_str(segment);
    *segments = next;
    Some(())
}

fn slice_ident<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    if !matches!(node.kind(), "identifier" | "crate" | "self" | "super") {
        return None;
    }
    let text = source.get(node.start_byte()..node.end_byte())?.trim();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
#[path = "rust_use_tree_tests.rs"]
mod tests;
