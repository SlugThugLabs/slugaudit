//! The synthetic frame a macro body is re-parsed in, and the mapping of its
//! byte offsets back to the original source.
use crate::evidence::calls::named_children;
use tree_sitter_language_pack::Node;

pub(super) const PREFIX: &str = "fn __slug_macro() { __slug_probe(";
pub(super) const SUFFIX: &str = "); }";

/// One reparsed fragment: the synthetic text, its frame in the source, and
/// how many macros it sits inside. Shared by every pending node of its tree.
pub(super) struct Fragment {
    pub(super) wrapped: String,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) macro_depth: u32,
}

/// True when the node's byte range sits inside the probe body of the
/// synthetic frame (not in the `PREFIX`/`SUFFIX` scaffolding itself).
pub(super) fn within_synthetic_frame(node: &Node, wrapped: &str) -> bool {
    node.start_byte() >= PREFIX.len()
        && wrapped
            .len()
            .checked_sub(SUFFIX.len())
            .is_some_and(|body_end| node.end_byte() <= body_end)
}

/// Maps a byte offset in a fragment's synthetic text back to the original
/// source, or `None` when the offset lies before the probe prefix or the sum
/// does not fit `usize`.
pub(super) fn absolute_offset(fragment_start: usize, synthetic_offset: usize) -> Option<usize> {
    fragment_start
        .checked_add(synthetic_offset)?
        .checked_sub(PREFIX.len())
}

/// A nested macro still owns an opaque token tree after the outer fragment is
/// reparsed. Returns its contents' range in absolute source coordinates, so
/// the scan reparses it with the caller in effect at the macro, or
/// `None` when `node` is not a macro with a non-empty body inside the frame.
pub(super) fn nested_macro_range(node: &Node, fragment: &Fragment) -> Option<(usize, usize)> {
    if node.kind() != "macro_invocation" {
        return None;
    }
    let tokens = named_children(node).find(|n| n.kind() == "token_tree")?;
    let start = absolute_offset(fragment.start, tokens.start_byte().checked_add(1)?)?;
    let end = absolute_offset(fragment.start, tokens.end_byte().checked_sub(1)?)?;
    (end <= fragment.end && start < end).then_some((start, end))
}
