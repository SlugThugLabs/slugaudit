//! The inputs and outputs of one structure run: how matches are rendered, the
//! stored layout of a match, the page cut applied while a stored page is read,
//! and the response assembled from a served page.

use super::page::{Completeness, Page, PageLimit};
use super::paginate::{Served, decode_rows};
use super::structure::stored::{PageCut, StoredMatch, StructureExtra};
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

/// What one run produced besides the stored matches.
pub(super) struct RunOutput {
    /// Files the scan finished (parsed or found unparseable) before it stopped.
    pub(super) files_scanned: usize,
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

/// How far the scan got, as stored with the result: `Partial` when the
/// execution budget cancelled a parse (resume after that file, which would
/// only time out again) or the memory window ended before the matching files
/// did (resume after the last file examined).
pub(super) fn completeness_of(window: &SourceWindow, run: &RunOutput) -> Completeness {
    let files_total = window
        .files
        .len()
        .saturating_add(window.files_over_parse_limit)
        .saturating_add(window.files_not_scanned) as u64;
    let files_scanned = run.files_scanned as u64;
    if let Some(file) = &run.parse_timed_out_file {
        return Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: Some(file.clone()),
            files_scanned,
            files_total,
        };
    }
    if window.files_not_scanned > 0 {
        return Completeness::Partial {
            limited_by: PageLimit::ScanWindow,
            resume_after_file: window.next_start_after_file.clone(),
            files_scanned,
            files_total,
        };
    }
    Completeness::Complete
}

/// The extras of a finished scan.
pub(super) fn extra_of(language: String, window: SourceWindow, run: RunOutput) -> StructureExtra {
    StructureExtra {
        language,
        files_not_scanned: window.files_not_scanned,
        bytes_not_scanned: window.bytes_not_scanned,
        next_start_after_file: run
            .parse_timed_out_file
            .clone()
            .or(window.next_start_after_file),
        unparsed_files: run.unparsed_files,
        files_over_parse_limit: window.files_over_parse_limit,
        parse_timed_out_file: run.parse_timed_out_file,
    }
}

/// Logs the call's facts and assembles the response from a served page.
/// `complete` is false whenever the stored scan was partial, even on the last
/// page, so a caller never reads a window-limited result as exhaustive.
pub(super) fn structure_response(
    served: Served<StructureExtra>,
    cut: &PageCut,
) -> Result<Json<StructureResponse>, ErrorData> {
    let stored = decode_rows::<StoredMatch>(&served.rows)?;
    let bytes_used = cut.bytes_used();
    let matches: Vec<StructureMatch> = stored.into_iter().map(|row| row.found).collect();
    let extra = served.meta;
    let more_pages = served.page.cursor.is_some();
    let partial_limit = match &served.page.completeness {
        Some(Completeness::Partial { limited_by, .. }) => Some(*limited_by),
        _ => None,
    };
    let limited_by = if more_pages {
        cut.limit()
    } else {
        partial_limit.unwrap_or(PageLimit::None)
    };
    tracing::info!(
        match_count = matches.len(),
        limited_by = ?limited_by,
        files_not_scanned = extra.files_not_scanned,
        unparsed = extra.unparsed_files.len(),
        over_parse_limit = extra.files_over_parse_limit,
        parse_timed_out = extra.parse_timed_out_file.is_some(),
        page_bytes = served.page.page_bytes,
        "structure executed"
    );
    let page = Page {
        // Counting the matches of a window-limited scan would be a lie about
        // the project; the stored count stays internal.
        total: None,
        complete: !more_pages && partial_limit.is_none(),
        bytes_used,
        limited_by,
        ..served.page
    };
    Ok(Json(StructureResponse {
        page,
        language: extra.language,
        matches,
        files_not_scanned: extra.files_not_scanned,
        bytes_not_scanned: extra.bytes_not_scanned,
        next_start_after_file: extra.next_start_after_file,
        unparsed_files: extra.unparsed_files,
        files_over_parse_limit: extra.files_over_parse_limit,
        parse_timed_out_file: extra.parse_timed_out_file,
    }))
}
