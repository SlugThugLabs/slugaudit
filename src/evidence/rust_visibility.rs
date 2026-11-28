//! Rust CST visibility-modifier extraction helper.

use super::{Extraction, named_children, node_text};
use tree_sitter::Node;

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
