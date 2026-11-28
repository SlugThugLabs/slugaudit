//! Rust `Comment` evidence classification helpers.

use super::rust_doc::is_outer_rust_doc_comment;

/// Classifies a Rust comment CST node as `"Doc"`, `"Line"`, or `"Block"`.
/// Both outer (`///`, `/**`) and inner (`//!`, `/*!`) doc comments classify as
/// `"Doc"` in `Comment` evidence rows.
pub(super) fn classify_rust_comment(kind: &str, text: &str) -> &'static str {
    if is_outer_rust_doc_comment(kind, text) || is_inner_rust_doc_comment(kind, text) {
        return "Doc";
    }
    if kind == "block_comment" {
        "Block"
    } else {
        "Line"
    }
}

fn is_inner_rust_doc_comment(kind: &str, text: &str) -> bool {
    (kind == "line_comment" && text.starts_with("//!"))
        || (kind == "block_comment" && text.starts_with("/*!"))
}
