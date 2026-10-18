//! The inputs and outputs of one structure run: how matches are rendered,
//! which slice of the ordered match stream is returned, and what came back.

use super::structure::StructureMatch;
use rmcp::ErrorData;
use tree_sitter::Query;

/// How a run reports its matches.
pub(super) struct OutputMode {
    /// Full matched text (up to `MAX_TEXT_BYTES`) instead of a 1-line snippet.
    pub(super) full_text: bool,
    /// A single-file run fails on an unparseable file; a multi-file run skips it.
    pub(super) single_file: bool,
}

/// Which slice of the ordered match stream one call returns.
pub(super) struct Paging {
    /// Matches to skip before the first returned one.
    pub(super) offset: usize,
    /// Serialized JSON bytes of `matches` at which the page is cut (the first
    /// match is always kept so every page makes progress).
    pub(super) page_bytes: usize,
}

/// What one run produced.
pub(super) struct RunOutput {
    pub(super) matches: Vec<StructureMatch>,
    /// More matches exist past this page (page bytes or match cap).
    pub(super) truncated: bool,
    /// Serialized JSON bytes of `matches`.
    pub(super) matches_bytes: usize,
    /// Multi-file only: files the parser produced no tree for.
    pub(super) unparsed_files: Vec<String>,
}

/// A bare pattern with no `@capture` can never return a node; reject it
/// with guidance instead of answering an empty, misleading success.
pub(super) fn require_captures(query: &Query) -> Result<(), ErrorData> {
    if query.capture_names().is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query declares no @capture, so it can never return any node; add "
                .to_owned()
                + "one, e.g. `(function_item name: (identifier) @name)` for Rust or "
                + "`(function_definition) @node` for Python/JS",
            None,
        ));
    }
    Ok(())
}
