//! Converting captured Tree-sitter nodes into bounded [`StructureMatch`]
//! coordinates and text/snippet payloads.

use super::StructureMatch;
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::{LineIndex, PositionOverflow, position_u32};
use rmcp::ErrorData;
use tree_sitter::{Query, QueryCapture, QueryMatch};

pub(super) const MAX_TEXT_BYTES: usize = 1_000_000;
const MAX_SNIPPET_BYTES: usize = 120;

/// How a run reports its matches.
pub(super) struct OutputMode {
    /// Full matched text (up to `MAX_TEXT_BYTES`) instead of a 1-line snippet.
    pub(super) full_text: bool,
    /// A single-file run fails on an unparseable file; a multi-file run skips it.
    pub(super) single_file: bool,
}

/// Rejects a bare pattern with no `@capture` instead of returning an empty
/// match list that looks like zero matches.
pub(super) fn require_captures(query: &Query) -> Result<(), ErrorData> {
    if query.capture_names().is_empty() {
        return Err(invalid_params_with_code(
            "structure query declares no @capture, so it can never return any node; add \
             one, e.g. `(function_item name: (identifier) @name)` for Rust or \
             `(function_definition) @node` for Python/JS",
            ErrorCode::InvalidTreeSitterQuery,
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
) -> Result<StructureMatch, ErrorData> {
    let (node_text, text_extraction_failed) = match node.utf8_text(content.as_bytes()) {
        Ok(text) => (text, false),
        Err(error) => {
            tracing::warn!(
                file = %file,
                start_byte = node.start_byte(),
                end_byte = node.end_byte(),
                error = %error,
                "structure: capture text is not valid UTF-8; reporting extraction_failed",
            );
            ("", true)
        }
    };
    let (capture_name, capture_name_missing) = if let Some(name) = capture_name {
        (name, false)
    } else {
        tracing::warn!(
            file = %file,
            "structure: capture has no name; reporting extraction_failed",
        );
        ("", true)
    };
    let (text, text_truncated) = if want_full_text {
        truncate_text(node_text)
    } else {
        truncate_snippet(node_text, MAX_SNIPPET_BYTES)
    };
    Ok(StructureMatch {
        file: file.to_owned(),
        capture_name: capture_name.to_owned(),
        node_kind: node.kind().to_owned(),
        start_byte: node.start_byte() as u64,
        end_byte: node.end_byte() as u64,
        start_line: position(one_based_row(node.start_position().row))?,
        start_column: position(lines.column(node.start_byte()))?,
        end_line: position(one_based_row(node.end_position().row))?,
        end_column: position(lines.column(node.end_byte()))?,
        text,
        text_truncated,
        extraction_failed: text_extraction_failed || capture_name_missing,
    })
}

fn one_based_row(zero_based: usize) -> Result<u32, PositionOverflow> {
    position_u32(
        zero_based
            .checked_add(1)
            .ok_or(PositionOverflow::Arithmetic {
                operation: "a one-based structure row",
            })?,
    )
}

fn position(value: Result<u32, PositionOverflow>) -> Result<u32, ErrorData> {
    value.map_err(|error| {
        ErrorData::internal_error(format!("structure match position: {error}"), None)
    })
}

pub(super) fn capture_at<'tree>(
    query_match: &QueryMatch<'_, 'tree>,
    index: usize,
) -> Result<QueryCapture<'tree>, ErrorData> {
    query_match.captures.get(index).copied().ok_or_else(|| {
        ErrorData::internal_error(
            format!(
                "tree-sitter reported capture index {index} but the match has {} captures",
                query_match.captures.len()
            ),
            None,
        )
    })
}

fn prefix_within(text: &str, max_bytes: usize) -> String {
    let mut room = max_bytes;
    let mut kept = String::new();
    for character in text.chars() {
        match room.checked_sub(character.len_utf8()) {
            Some(left) => room = left,
            None => break,
        }
        kept.push(character);
    }
    kept
}

fn truncate_text(text: &str) -> (String, bool) {
    if text.len() <= MAX_TEXT_BYTES {
        return (text.to_owned(), false);
    }
    (prefix_within(text, MAX_TEXT_BYTES), true)
}

fn truncate_snippet(text: &str, max_bytes: usize) -> (String, bool) {
    let first_line = match text.lines().next() {
        Some(line) => line.trim(),
        None => "",
    };
    if first_line.len() <= max_bytes {
        (first_line.to_owned(), text.lines().nth(1).is_some())
    } else {
        (prefix_within(first_line, max_bytes), true)
    }
}
