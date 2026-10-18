//! The inputs and outputs of one structure run: how matches are rendered,
//! which slice of the ordered match stream is returned, and what came back.

use super::structure::StructureMatch;
use super::structure_source::{MAX_SNIPPET_BYTES, truncate_snippet, truncate_text};
use crate::model::{LineIndex, saturating_u32};
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

/// Maps a captured node back to source coordinates and bounded text.
pub(super) fn node_to_match(
    file: &str,
    node: tree_sitter::Node<'_>,
    content: &str,
    lines: &LineIndex<'_>,
    capture_name: Option<&str>,
    want_full_text: bool,
) -> StructureMatch {
    let (node_text, text_extraction_failed) = match node.utf8_text(content.as_bytes()) {
        Ok(text) => (text, false),
        Err(error) => {
            tracing::warn!(
                file_id = %crate::util::redacted_path(std::path::Path::new(file)),
                start_byte = node.start_byte(),
                end_byte = node.end_byte(),
                error = %error,
                "structure: capture text is not valid UTF-8; reporting extraction_failed",
            );
            ("", true)
        }
    };
    if capture_name.is_none() {
        tracing::warn!(
            file_id = %crate::util::redacted_path(std::path::Path::new(file)),
            "structure: capture has no name; reporting extraction_failed",
        );
    }
    let (text, text_truncated) = if want_full_text {
        truncate_text(node_text)
    } else {
        truncate_snippet(node_text, MAX_SNIPPET_BYTES)
    };
    StructureMatch {
        file: file.to_owned(),
        capture_name: capture_name.unwrap_or_default().to_owned(),
        node_kind: node.kind().to_owned(),
        start_byte: node.start_byte() as u64,
        end_byte: node.end_byte() as u64,
        start_line: saturating_u32(node.start_position().row),
        start_column: lines.column(node.start_byte()),
        end_line: saturating_u32(node.end_position().row),
        end_column: lines.column(node.end_byte()),
        text,
        text_truncated,
        extraction_failed: text_extraction_failed || capture_name.is_none(),
    }
}
