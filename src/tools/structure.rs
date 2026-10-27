//! The `structure` tool contract: request/response types, input
//! validation, and target loading. Query execution lives in
//! `structure_exec`.

use super::page::{LegacyPaging, Page};
use super::page_size;
use super::paginate::{PageSpec, page_request, paginate_with};
use super::structure_exec::produce;
use super::structure_page::structure_response;
use crate::model::{ResourceLimits, process_limits};
use crate::store::StoredRow;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use stored::{PageCut, StoredMatch};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILTER_BYTES: usize = 8 * 1024;

use super::structure_source::MAX_MULTI_FILE_TOTAL_BYTES;
#[cfg(test)]
pub(super) use super::structure_source::MAX_TEXT_BYTES;

#[derive(Debug, Default, Deserialize, JsonSchema)]
/// A Tree-sitter query to run against one indexed file, or every indexed
/// file of one language. The matches are stored when the call runs; the
/// response's `page.cursor` pages through that stored answer.
pub struct StructureRequest {
    /// Any path inside the active project. Required without `cursor`;
    /// ignored with one.
    #[serde(default)]
    pub path: String,
    /// The `page.cursor` of a previous response, copied exactly. It
    /// continues that stored result: nothing is synced or re-scanned, and
    /// `path` and every other request-shaping parameter are ignored
    /// (`page_bytes` still applies).
    #[serde(default)]
    pub cursor: Option<String>,
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
    /// At most `max_structure_query_bytes` UTF-8 bytes (16384 by default;
    /// `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES`, or `max_structure_query_bytes`
    /// in the project's `config.json`); a longer query is refused with
    /// `invalid_params` before it is compiled.
    pub query: String,
    /// Whether to include full matched source text up to MAX_TEXT_BYTES.
    /// Defaults to true for single-file queries, and false (lean 1-line snippet) for multi-file queries.
    #[serde(default)]
    pub full_text: Option<bool>,
    /// Page size in serialized JSON bytes of `matches`. Omit it for the server
    /// default (64 KiB unless configured otherwise; the response's
    /// `page.page_bytes` says what applied). A value above the hard ceiling
    /// `max_query_response_bytes` is clamped, and a page the ceiling cut
    /// reports `page.limited_by` = `ceiling`. Pages are cut on a match
    /// boundary; the first
    /// match is always returned, so a single match larger than the page
    /// arrives whole (its own `text_truncated` flag covers the 1 MB text cap).
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// Multi-file queries only: scan only files whose path sorts strictly
    /// after this one. To scan the files a window-limited result did not
    /// reach, make a new request with `start_after_file` set to the result's
    /// `resume_after_file` (also reported as `next_start_after_file`); it
    /// syncs and describes the current tree.
    #[serde(default)]
    pub start_after_file: Option<String>,
    /// Refuses the removed `offset` and `expected_revision` parameters.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
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
    /// Paging for `matches`. `cursor` pages through the stored result;
    /// `completeness` says whether the scan behind it covered everything
    /// (`partial` names `resume_after_file`). `complete` means no page and no
    /// scan window remains. `total` is always null (counting needs the whole
    /// scan).
    pub page: Page,
    /// The grammar this query ran against.
    pub language: String,
    /// This page's matches in scan order. The first match is always returned,
    /// so paging always advances.
    pub matches: Vec<StructureMatch>,
    /// Multi-file only: matching files past the memory window that were not
    /// loaded into this call's scan window, so certainly not scanned. Zero
    /// means every matching file after `start_after_file` was loaded into the
    /// window. When above 0 the stored result is `partial` and
    /// `page.complete` is false: read it to the end with `page.cursor`, then
    /// make a new request with `start_after_file` = `next_start_after_file`.
    pub files_not_scanned: usize,
    /// Content bytes of those unscanned files.
    pub bytes_not_scanned: u64,
    /// The `start_after_file` for the new request that scans past this
    /// result (the result's `resume_after_file`). Present when
    /// `files_not_scanned` > 0 (`completeness.limited_by` = `scan_window`),
    /// or when `parse_timed_out_file` is set (then it is that file, so the
    /// new request skips it; `completeness.limited_by` = `time_budget`).
    pub next_start_after_file: Option<String>,
    /// Multi-file only: files the parser produced no syntax tree for, so they
    /// contributed no matches. Empty when none.
    pub unparsed_files: Vec<String>,
    /// Multi-file only: matching files in this window's path range larger
    /// than `max_parse_bytes` (or the 64 MiB scan window, if smaller). They
    /// were skipped without being loaded and contributed no matches; their
    /// content is still searchable via `query`. When above 0 the result does
    /// not cover every matching file.
    pub files_over_parse_limit: usize,
    /// Multi-file only: set when the execution time budget ran out while this
    /// file was being parsed. The parse was cancelled, the scan stopped, and
    /// `matches` holds only what earlier files produced: the result is
    /// partial. Make a new request with `start_after_file` =
    /// `next_start_after_file` (this file) to continue past it. A single-file
    /// query whose parse runs out of time is refused with the
    /// execution-budget error instead.
    pub parse_timed_out_file: Option<String>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `request.file`
/// isn't indexed with a detected, pack-supported language, the query text
/// is empty/too large or fails to compile, the parser returns no tree, the
/// request carries the removed `offset` or `expected_revision`, or `cursor`
/// names no readable stored result.
pub fn structure(
    request: &Parameters<StructureRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let request_path = request.0.path.as_str();
    // A continuation ignores `path`, so it has no project to read limits from;
    // its page budget comes from the stored result, these limits only supply
    // the ceiling and the match cap.
    let limits = if request.0.cursor.is_none() && !request_path.is_empty() {
        super::resolve_limits(request_path)?
    } else {
        *process_limits()
    };
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
    request.legacy_paging.refuse()?;
    let page = page_size::resolve(request.page_bytes, limits)?;
    let path = (!request.path.is_empty()).then_some(request.path.as_str());
    let page_request = page_request(request.cursor.as_deref(), path, sink)?;
    match request.cursor.as_deref() {
        Some(cursor) => validate_cursor(cursor)?,
        None => validate_request(request, limits)?,
    }
    let mut spec = PageSpec::typed::<StoredMatch>("structure", page.bytes);
    spec.offer_first_row = true;
    spec.requested_page_bytes = request.page_bytes.map(|_| page.bytes);
    let ceiling = limits.max_query_response_bytes;
    let max_matches = limits.max_structure_matches;
    let mut cut = PageCut::new(page.bytes, ceiling, max_matches);
    let served = paginate_with(
        manager,
        &spec,
        page_request,
        |tx, rows| produce(tx, rows, request, limits, window_bytes),
        |effective| {
            cut = PageCut::new(effective, ceiling, max_matches);
            |row: &StoredRow| cut.accept(row)
        },
    )?;
    cut.check()?;
    structure_response(served, &cut)
}

/// Bounds the cursor like every other string input.
fn validate_cursor(cursor: &str) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(cursor, MAX_FILTER_BYTES) {
        return Err(ErrorData::invalid_params(
            format!("cursor exceeds {MAX_FILTER_BYTES} bytes"),
            None,
        ));
    }
    Ok(())
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
    ] {
        if value.is_some_and(|value| crate::util::exceeds_input_bytes(value, max_bytes)) {
            return Err(ErrorData::invalid_params(
                format!("{name} exceeds {max_bytes} bytes"),
                None,
            ));
        }
    }
    if request
        .language
        .as_deref()
        .is_some_and(crate::parse::is_prose_language_name)
    {
        return Err(super::structure_source::prose_not_parsed_error());
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
                "structure query is {} bytes; the limit is {} (max_structure_query_bytes)",
                request.query.len(),
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

#[path = "structure_stored.rs"]
pub(super) mod stored;

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "structure_limit_tests.rs"]
mod limit_tests;

#[cfg(test)]
#[path = "structure_parse_bounds_tests.rs"]
mod parse_bounds_tests;

#[cfg(test)]
#[path = "structure_language_tests.rs"]
mod language_tests;

#[cfg(test)]
#[path = "structure_paged_tests.rs"]
mod paged_tests;
