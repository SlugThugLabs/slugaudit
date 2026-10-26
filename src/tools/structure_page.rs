//! The inputs and outputs of one structure run: how matches are rendered,
//! which slice of the ordered match stream is returned, and what came back.

use super::page_size::PageSize;
use super::structure::{StructureMatch, StructureResponse};
use super::structure_source::{MAX_SNIPPET_BYTES, SourceWindow, truncate_snippet, truncate_text};
use crate::model::{LineIndex, saturating_u32};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Json;
use tree_sitter::{Query, QueryCapture, QueryMatch};

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
    /// Multi-file only: the file whose parse the deadline cancelled.
    pub(super) parse_timed_out_file: Option<String>,
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
        start_line: one_based_row(node.start_position().row),
        start_column: lines.column(node.start_byte()),
        end_line: one_based_row(node.end_position().row),
        end_column: lines.column(node.end_byte()),
        text,
        text_truncated,
        extraction_failed: text_extraction_failed || capture_name.is_none(),
    }
}

/// The single zero-based to one-based line conversion for structure matches
/// (tree-sitter rows are zero-based; the response is one-based, inclusive).
/// An overflowing row is clamped and logged by `saturating_u32`.
fn one_based_row(zero_based: usize) -> u32 {
    saturating_u32(zero_based.saturating_add(1))
}

/// The capture at `index`, or the internal error for an index the match does
/// not hold.
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

/// Maps one capture back to a match and measures its serialized size.
pub(super) fn capture_to_match(
    file_path: &str,
    content: &str,
    lines: &LineIndex<'_>,
    capture: QueryCapture<'_>,
    capture_name: Option<&str>,
    full_text: bool,
) -> Result<(StructureMatch, usize), ErrorData> {
    let found = node_to_match(
        file_path,
        capture.node,
        content,
        lines,
        capture_name,
        full_text,
    );
    let found_bytes = serde_json::to_vec(&found)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .len();
    Ok((found, found_bytes))
}

/// Logs the run's facts and assembles the response, including the paging
/// continuation for a cut page.
pub(super) fn structure_response(
    revision_id: String,
    language: String,
    window: SourceWindow,
    run: RunOutput,
    page: PageSize,
    offset: usize,
) -> Json<StructureResponse> {
    let next_offset = run
        .truncated
        .then_some(offset.saturating_add(run.matches.len()));
    tracing::info!(
        match_count = run.matches.len(),
        truncated = run.truncated,
        files_not_scanned = window.files_not_scanned,
        unparsed = run.unparsed_files.len(),
        over_parse_limit = window.files_over_parse_limit,
        parse_timed_out = run.parse_timed_out_file.is_some(),
        page_bytes = page.bytes,
        clamped = page.clamped_from.is_some(),
        "structure executed"
    );

    Json(StructureResponse {
        revision_id,
        language,
        matches: run.matches,
        truncated: run.truncated,
        next_offset,
        page_bytes: page.bytes,
        page_bytes_clamped_from: page.clamped_from,
        matches_bytes: run.matches_bytes,
        files_not_scanned: window.files_not_scanned,
        bytes_not_scanned: window.bytes_not_scanned,
        next_start_after_file: run
            .parse_timed_out_file
            .clone()
            .or(window.next_start_after_file),
        unparsed_files: run.unparsed_files,
        files_over_parse_limit: window.files_over_parse_limit,
        parse_timed_out_file: run.parse_timed_out_file,
    })
}
