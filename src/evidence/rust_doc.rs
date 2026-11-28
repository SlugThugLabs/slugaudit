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

/// True when `kind` is a Rust comment or outer doc-comment CST node.
pub(super) fn is_rust_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "line_comment" | "block_comment" | "line_outer_doc_comment" | "block_outer_doc_comment"
    )
}

/// Returns the first non-extra sibling after `node`'s leading `attributes`
/// child, or `node` itself when no `attributes` child is present.
pub(super) fn declaration_start_node(node: Node<'_>) -> Node<'_> {
    let Some(attributes) = named_children(node).find(|child| child.kind() == "attributes") else {
        return node;
    };
    let mut next = attributes.next_sibling();
    while let Some(sibling) = next {
        if !sibling.is_extra() {
            return sibling;
        }
        next = sibling.next_sibling();
    }
    node
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
        self.clear_if_gap(child.start_position().row);
        match child.kind() {
            kind if is_rust_comment_kind(kind) => {
                self.push_comment(child, source, out);
                None
            }
            "attribute_item" => {
                self.push_attr(child, source);
                None
            }
            "inner_attribute_item" => {
                self.clear_docs();
                self.attrs.clear();
                None
            }
            _ => {
                if let Some(attrs) = named_children(child).find(|node| node.kind() == "attributes")
                {
                    self.push_attributes_node(attrs, source, out);
                    let start = declaration_start_node(child);
                    let mut extra = attrs.next_sibling();
                    while let Some(sib) = extra.filter(|s| s.id() != start.id()) {
                        if sib.is_extra() && is_rust_comment_kind(sib.kind()) {
                            self.push_comment(sib, source, out);
                        }
                        extra = sib.next_sibling();
                    }
                    self.clear_if_gap(start.start_position().row);
                }
                Some(NodeMeta {
                    outer_doc: self.take_doc(),
                    outer_attrs: std::mem::take(&mut self.attrs),
                })
            }
        }
    }

    pub(super) fn push_attributes_node(
        &mut self,
        attrs: Node<'tree>,
        source: &str,
        out: &mut Extraction,
    ) {
        for attr_child in named_children(attrs) {
            self.clear_if_gap(attr_child.start_position().row);
            if is_rust_comment_kind(attr_child.kind()) {
                self.push_comment(attr_child, source, out);
            } else if attr_child.kind() == "attribute_item" {
                self.push_attr(attr_child, source);
            }
        }
    }

    fn clear_if_gap(&mut self, row: usize) {
        let gap = self
            .doc_last_end_row
            .is_some_and(|prev| prev.checked_add(1).is_none_or(|next| row > next));
        if gap {
            self.clear_docs();
        }
    }

    fn push_comment(&mut self, child: Node<'tree>, source: &str, out: &mut Extraction) {
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
    }

    fn push_attr(&mut self, child: Node<'tree>, source: &str) {
        self.attrs.push(child);
        if !self.doc_lines.is_empty() {
            self.doc_last_end_row = Some(content_end_row(child, source));
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
    matches!(kind, "line_outer_doc_comment" | "block_outer_doc_comment")
        || (kind == "line_comment" && text.starts_with("///") && !text.starts_with("////"))
        || (kind == "block_comment"
            && text.starts_with("/**")
            && text != "/**/"
            && !text.starts_with("/***"))
}

/// Last 0-based source row containing real bytes of `node`, excluding any
/// trailing `\n` that `tree-sitter-rust`'s `line_comment` token swallowed.
pub(super) fn content_end_row(node: Node<'_>, source: &str) -> usize {
    let (end_byte, end_row) = (node.end_byte(), node.end_position().row);
    let has_trailing_nl = end_byte > node.start_byte()
        && end_byte
            .checked_sub(1)
            .and_then(|last| source.as_bytes().get(last))
            == Some(&b'\n');
    if has_trailing_nl && let Some(prev) = end_row.checked_sub(1) {
        return prev;
    }
    end_row
}
