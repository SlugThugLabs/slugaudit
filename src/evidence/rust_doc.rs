//! Shared Rust CST helpers for visibility modifiers, outer `#[...]` attribute
//! decorators, outer `///` / `/** */` doc comments (excluding inner `//!` and
//! `/*!` module-level docs), and `Comment` evidence classification.

use super::{Extraction, named_children, node_text};
use tree_sitter::Node;

/// Preceding outer doc comment and outer `#[...]` attribute nodes paired with a
/// Rust CST child node.
#[derive(Default)]
pub(super) struct NodeMeta<'tree> {
    pub(super) outer_doc: Option<String>,
    pub(super) outer_attrs: Vec<Node<'tree>>,
}

impl NodeMeta<'_> {
    /// Slices and trims all attached outer `#[...]` attributes in source order.
    pub(super) fn decorator_strings(&self, source: &str, out: &mut Extraction) -> Vec<String> {
        let mut decorators = Vec::with_capacity(self.outer_attrs.len());
        for &attr in &self.outer_attrs {
            match node_text(source, attr.start_byte(), attr.end_byte()) {
                Ok(text) => {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        decorators.push(trimmed.to_owned());
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "Rust outer attribute skipped; extraction is partial");
                    out.truncated = true;
                }
            }
        }
        decorators
    }
}

/// Extracts the trimmed `visibility_modifier` slice (`"pub"`, `"pub(crate)"`,
/// `"pub(super)"`, `"pub(in ...)"`) of a Rust item or field `node`, or `None`
/// when the declaration is private.
pub(super) fn rust_visibility(
    node: Node<'_>,
    source: &str,
    out: &mut Extraction,
) -> Option<String> {
    let vis_node = named_children(node).find(|child| child.kind() == "visibility_modifier")?;
    match node_text(source, vis_node.start_byte(), vis_node.end_byte()) {
        Ok(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Err(error) => {
            tracing::warn!(%error, "Rust visibility modifier skipped; extraction is partial");
            out.truncated = true;
            None
        }
    }
}

/// Classifies a Rust `line_comment` or `block_comment` as `"Doc"`, `"Line"`, or
/// `"Block"`. Both outer (`///`, `/**`) and inner (`//!`, `/*!`) doc comments
/// classify as `"Doc"` in `Comment` evidence rows.
pub(super) fn classify_rust_comment(kind: &str, text: &str) -> &'static str {
    if is_outer_rust_doc_comment(kind, text) || is_inner_rust_doc_comment(kind, text) {
        "Doc"
    } else if kind == "block_comment" {
        "Block"
    } else {
        "Line"
    }
}

/// Accumulates adjacent outer Rust doc comments (`///` and `/** ... */`) and
/// preceding outer `#[...]` (`attribute_item`) nodes in a single forward pass
/// over a parent node's named children.
///
/// Inner doc comments (`//!` and `/*!`) and inner attributes (`#![...]`) never
/// attach to items and reset any pending outer doc comment.
#[derive(Default)]
pub(super) struct SiblingTracker<'tree> {
    doc_lines: Vec<String>,
    doc_last_end_row: Option<usize>,
    attrs: Vec<Node<'tree>>,
}

impl<'tree> SiblingTracker<'tree> {
    pub(super) fn step(
        &mut self,
        child: Node<'tree>,
        source: &str,
        out: &mut Extraction,
    ) -> Option<NodeMeta<'tree>> {
        if let Some(prev_row) = self.doc_last_end_row {
            let adjacent = prev_row
                .checked_add(1)
                .is_some_and(|next_row| child.start_position().row <= next_row);
            if !adjacent {
                self.clear_docs();
            }
        }
        match child.kind() {
            "line_comment" | "block_comment" => {
                match node_text(source, child.start_byte(), child.end_byte()) {
                    Ok(text) if is_outer_rust_doc_comment(child.kind(), text) => {
                        self.doc_lines.push(text.trim_end().to_owned());
                        self.doc_last_end_row = Some(content_end_row(child, source));
                    }
                    Ok(_) => self.clear_docs(),
                    Err(error) => {
                        tracing::warn!(%error, "Rust comment skipped; extraction is partial");
                        out.truncated = true;
                        self.clear_docs();
                    }
                }
                None
            }
            "attribute_item" => {
                self.attrs.push(child);
                if !self.doc_lines.is_empty() {
                    self.doc_last_end_row = Some(content_end_row(child, source));
                }
                None
            }
            "inner_attribute_item" => {
                self.clear_docs();
                self.attrs.clear();
                None
            }
            _ => Some(NodeMeta {
                outer_doc: self.take_doc(),
                outer_attrs: std::mem::take(&mut self.attrs),
            }),
        }
    }

    fn take_doc(&mut self) -> Option<String> {
        if self.doc_lines.is_empty() {
            self.doc_last_end_row = None;
            return None;
        }
        let joined = self.doc_lines.join("\n");
        self.clear_docs();
        Some(joined)
    }

    fn clear_docs(&mut self) {
        self.doc_lines.clear();
        self.doc_last_end_row = None;
    }
}

pub(super) fn is_outer_rust_doc_comment(kind: &str, text: &str) -> bool {
    let is_line_doc =
        kind == "line_comment" && text.starts_with("///") && !text.starts_with("////");
    let is_block_doc = kind == "block_comment"
        && text.starts_with("/**")
        && text != "/**/"
        && !text.starts_with("/***");
    is_line_doc || is_block_doc
}

fn is_inner_rust_doc_comment(kind: &str, text: &str) -> bool {
    (kind == "line_comment" && text.starts_with("//!"))
        || (kind == "block_comment" && text.starts_with("/*!"))
}

/// Last 0-based source row containing real bytes of `node`, excluding any
/// trailing `\n` that `tree-sitter-rust`'s `line_comment` token swallowed.
pub(super) fn content_end_row(node: Node<'_>, source: &str) -> usize {
    let end_byte = node.end_byte();
    let end_row = node.end_position().row;
    if end_byte > node.start_byte()
        && end_byte
            .checked_sub(1)
            .and_then(|last| source.as_bytes().get(last))
            == Some(&b'\n')
        && let Some(prev) = end_row.checked_sub(1)
    {
        return prev;
    }
    end_row
}
