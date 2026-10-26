//! One tree-sitter parse that stops when a budget is spent. `structure`, the
//! registered-mechanic scans and the call queries all parse under a deadline
//! through this function, so the cancel-and-reset rule lives in one place.

use std::ops::ControlFlow;
use tree_sitter::{ParseOptions, ParseState, Parser, Tree};

/// What one budgeted parse produced.
pub(crate) enum Parsed {
    /// The parse finished.
    Tree(Tree),
    /// The progress callback stopped the parse because `spent` said so.
    Cancelled,
    /// The parser returned no tree for another reason.
    NoTree,
}

/// Parses `content` through tree-sitter 0.26's `Parser::parse_with_options` +
/// `ParseOptions::progress_callback` (the callback runs periodically during
/// the parse; `ControlFlow::Break` halts it and the call returns `None`).
/// `spent` says whether the caller's budget is gone; it is asked on every
/// callback. A halted parser keeps its partial state to resume, so it is
/// reset before it is used again. Verified against tree-sitter 0.26.13
/// (`binding_rust/lib.rs`), 2026-10-25.
pub(crate) fn parse_within(parser: &mut Parser, content: &str, spent: impl Fn() -> bool) -> Parsed {
    let bytes = content.as_bytes();
    let mut read = |offset: usize, _| bytes.get(offset..).unwrap_or_default();
    let mut cancelled = false;
    let mut check_budget = |_state: &ParseState| -> ControlFlow<()> {
        if spent() {
            cancelled = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut check_budget);
    let tree = parser.parse_with_options(&mut read, None, Some(options));
    match tree {
        Some(tree) => Parsed::Tree(tree),
        None if cancelled => {
            parser.reset();
            Parsed::Cancelled
        }
        None => Parsed::NoTree,
    }
}
