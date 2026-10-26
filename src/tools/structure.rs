//! The `structure` tool contract: request/response types, input
//! validation, and target loading. Query execution lives in
//! `structure_exec`.

use super::context::{ensure_synced, guard_expected_revision};
use super::page_size;
use super::structure_exec::run_query;
use super::structure_page::{OutputMode, Paging, structure_response};
use crate::model::ResourceLimits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::Query;

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILTER_BYTES: usize = 8 * 1024;
const MAX_REVISION_BYTES: usize = 256;

#[cfg(test)]
pub(super) use super::structure_source::MAX_TEXT_BYTES;
use super::structure_source::{MAX_MULTI_FILE_TOTAL_BYTES, load_targets};

#[derive(Debug, Default, Deserialize, JsonSchema)]
/// A Tree-sitter query to run against one indexed file, or every indexed
/// file of one language.
pub struct StructureRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Project-relative path of a single file to match against.
    /// When omitted, matches all files in the project matching `language`.
    #[serde(default)]
    pub file: Option<String>,
    /// Language grammar to use (e.g. "rust", "python", "typescript").
    /// Required when `file` is omitted; optional if `file` is provided.
    #[serde(default)]
    pub language: Option<String>,
    /// Optional path pattern (e.g. "*auth*", "src/**/*.rs") to filter files.
    #[serde(default)]
    pub pattern: Option<String>,
    /// A tree-sitter S-expression query, e.g. `(function_item name: (identifier) @name)`.
    /// For patterns normalized evidence and `query` can't easily express.
    pub query: String,
    /// Whether to include full matched source text up to MAX_TEXT_BYTES.
    /// Defaults to true for single-file queries, and false (lean 1-line snippet) for multi-file queries.
    #[serde(default)]
    pub full_text: Option<bool>,
    /// Number of matches to skip when paging through a result; use the
    /// previous response's `next_offset`. Valid only while `revision_id` is
    /// unchanged, and only together with the same `start_after_file`.
    #[serde(default)]
    pub offset: usize,
    /// Page size in serialized JSON bytes of `matches`. Omit it for the server
    /// default (64 KiB unless configured otherwise; the response's
    /// `page_bytes` says what applied). A value above the hard ceiling
    /// `max_query_response_bytes` is clamped and reported as
    /// `page_bytes_clamped_from`. Pages are cut on a match boundary; the first
    /// match is always returned, so a single match larger than the page
    /// arrives whole (its own `text_truncated` flag covers the 1 MB text cap).
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// Multi-file queries only: scan only files whose path sorts strictly
    /// after this one. Use the previous response's `next_start_after_file`
    /// when it reports `files_not_scanned`.
    #[serde(default)]
    pub start_after_file: Option<String>,
    /// Set to a previous page's `revision_id` when resuming; if the index
    /// changed since, the call is refused instead of paging a shifted order.
    #[serde(default)]
    pub expected_revision: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One capture match mapped back to source coordinates, with the
/// clipping/extraction flags that bound its `text`.
pub struct StructureMatch {
    pub file: String,
    pub capture_name: String,
    pub node_kind: String,
    pub start_byte: u64,
    pub end_byte: u64,
    /// One-based, inclusive line where the match starts.
    pub start_line: u32,
    pub start_column: u32,
    /// One-based, inclusive line where the match ends.
    pub end_line: u32,
    pub end_column: u32,
    pub text: String,
    pub text_truncated: bool,
    pub extraction_failed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
/// The matches for one structure call, the revision they were read from,
/// and every cut that bounded them, each with the way past it.
pub struct StructureResponse {
    /// Revision id the matches were read from; pass it back as
    /// `expected_revision` when paging.
    pub revision_id: String,
    /// The grammar this query ran against.
    pub language: String,
    /// This page's matches in scan order. The first match is always returned,
    /// so paging always advances.
    pub matches: Vec<StructureMatch>,
    /// True when more matches exist past this page (page bytes or the match
    /// cap); `next_offset` continues. False means this scan window is done.
    pub truncated: bool,
    /// Pass as the next request's `offset` (with the same other fields) when
    /// present.
    pub next_offset: Option<usize>,
    /// The page size that applied, in serialized JSON bytes of `matches`.
    pub page_bytes: usize,
    /// Non-null when the requested (or configured default) page size exceeded
    /// `max_query_response_bytes`: the size asked for; `page_bytes` is then
    /// the ceiling.
    pub page_bytes_clamped_from: Option<usize>,
    /// Serialized JSON bytes of the returned `matches`.
    pub matches_bytes: usize,
    /// Multi-file only: matching files past the memory window that were not
    /// loaded into this call's scan window, so certainly not scanned. Zero
    /// means every matching file after `start_after_file` was loaded into the
    /// window — it does not mean every loaded file was scanned, because a page
    /// cut (`truncated` true) can stop inside the window and leave later
    /// loaded files contributing nothing to this page. When above 0 the result
    /// is incomplete even if `truncated` is false: finish `next_offset` paging,
    /// then repeat with `start_after_file` = `next_start_after_file` and offset
    /// 0.
    pub files_not_scanned: usize,
    /// Content bytes of those unscanned files.
    pub bytes_not_scanned: u64,
    /// Present when `files_not_scanned` > 0.
    pub next_start_after_file: Option<String>,
    /// Multi-file only: files the parser produced no syntax tree for, so they
    /// contributed no matches. Empty when none.
    pub unparsed_files: Vec<String>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `request.file`
/// isn't indexed with a detected, pack-supported language, the query text
/// is empty/too large or fails to compile, or the parser returns no tree.
pub fn structure(
    request: &Parameters<StructureRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let limits = super::resolve_limits(&request.0.path)?;
    structure_with_limits(request, &limits, MAX_MULTI_FILE_TOTAL_BYTES, sink, manager)
}

/// Test-only seam: production code always goes through [`structure`] with
/// `process_limits`; tests inject a tighter `max_structure_execution_time`
/// to exercise the abort path without waiting out the production-sized
/// budget, and a small `window_bytes` to cut the multi-file memory window
/// without writing 64 MB.
fn structure_with_limits(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    window_bytes: usize,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let request = &request.0;
    validate_request(request, limits)?;
    let page = page_size::resolve(request.page_bytes, limits)?;

    let synced = ensure_synced(&request.path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, request.expected_revision.as_deref())?;
    let (target_language, window) = load_targets(&synced, request, window_bytes)?;
    let (ts_language, compiled_query) = compile_query(&target_language, &request.query)?;

    let single_file = request.file.is_some();
    let mode = OutputMode {
        full_text: request.full_text.unwrap_or(single_file),
        single_file,
    };
    let paging = Paging {
        offset: request.offset,
        page_bytes: page.bytes,
    };
    let run = run_query(
        &compiled_query,
        &ts_language,
        &window.files,
        limits,
        &mode,
        &paging,
    )?;
    Ok(structure_response(
        synced.revision_id,
        target_language,
        window,
        run,
        page,
        request.offset,
    ))
}

/// Compiles the requested grammar and query, mapping failures to contract
/// errors.
fn compile_query(language: &str, text: &str) -> Result<(tree_sitter::Language, Query), ErrorData> {
    let ts_language = tree_sitter_language_pack::get_language(language)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let compiled_query = Query::new(&ts_language, text)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    Ok((ts_language, compiled_query))
}

/// Rejects oversized or empty inputs before any sync or parse work.
fn validate_request(request: &StructureRequest, limits: &ResourceLimits) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    for (name, value, max_bytes) in [
        ("file", request.file.as_deref(), MAX_FILTER_BYTES),
        ("language", request.language.as_deref(), MAX_FILTER_BYTES),
        ("pattern", request.pattern.as_deref(), MAX_FILTER_BYTES),
        (
            "start_after_file",
            request.start_after_file.as_deref(),
            MAX_PATH_BYTES,
        ),
        (
            "expected_revision",
            request.expected_revision.as_deref(),
            MAX_REVISION_BYTES,
        ),
    ] {
        if value.is_some_and(|value| crate::util::exceeds_input_bytes(value, max_bytes)) {
            return Err(ErrorData::invalid_params(
                format!("{name} exceeds {max_bytes} bytes"),
                None,
            ));
        }
    }
    if request.query.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query must not be empty",
            None,
        ));
    }
    if request.query.len() > limits.max_structure_query_bytes {
        return Err(ErrorData::invalid_params(
            format!(
                "structure query exceeds {} bytes",
                limits.max_structure_query_bytes
            ),
            None,
        ));
    }
    if request.file.is_some() && request.start_after_file.is_some() {
        return Err(ErrorData::invalid_params(
            "start_after_file applies only to multi-file queries; omit it with `file`",
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "structure_limit_tests.rs"]
mod limit_tests;
