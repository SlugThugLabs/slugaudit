//! Request and response schema types for the `structure` tool.

use super::super::page::{LegacyPaging, Page};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// At most `max_structure_query_bytes` UTF-8 bytes (16384 by default).
    /// Required without `cursor`; ignored with one.
    #[serde(default)]
    pub query: String,
    /// Whether to include full matched source text up to `MAX_TEXT_BYTES`.
    /// Defaults to true for single-file queries, and false (lean 1-line snippet) for multi-file queries.
    #[serde(default)]
    pub full_text: Option<bool>,
    /// Page size in serialized JSON bytes of `matches`. Omit it for the server
    /// default (64 KiB unless configured otherwise; `page.page_bytes` says
    /// what applied). Clamped to `max_query_response_bytes`. Pages are cut on
    /// exact match boundaries at `min(page_bytes, max_structure_matches)`.
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// Multi-file queries only: scan only files whose path sorts strictly
    /// after this one.
    #[serde(default)]
    pub start_after_file: Option<String>,
    /// Refuses the `offset` and `expected_revision` parameters.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One capture match mapped back to source coordinates, with the
/// clipping/extraction flags that bound its `text`.
pub struct StructureMatch {
    /// Project-relative file path.
    pub file: String,
    /// Capture name without `@`.
    pub capture_name: String,
    /// Tree-sitter node kind.
    pub node_kind: String,
    /// Zero-based inclusive start byte offset.
    pub start_byte: u64,
    /// Zero-based exclusive end byte offset.
    pub end_byte: u64,
    /// One-based, inclusive line where the match starts.
    pub start_line: u32,
    /// Zero-based character column on `start_line`.
    pub start_column: u32,
    /// One-based, inclusive line where the match ends.
    pub end_line: u32,
    /// Zero-based character column on `end_line`.
    pub end_column: u32,
    /// Matched text or 1-line snippet.
    pub text: String,
    /// True when `text` was truncated to fit the text/snippet cap or spans multiple lines in snippet mode.
    pub text_truncated: bool,
    /// True when UTF-8 decoding or capture naming failed.
    pub extraction_failed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
/// The matches for one structure call, the revision they were read from,
/// and every cut that bounded them, each with the way past it.
pub struct StructureResponse {
    /// Paging for `matches`. `cursor` pages through the stored result;
    /// `completeness` says whether the scan behind it covered everything.
    pub page: Page,
    /// The grammar this query ran against.
    pub language: String,
    /// One line when `language` is untested; empty otherwise.
    pub notices: Vec<String>,
    /// This page's matches in scan order.
    pub matches: Vec<StructureMatch>,
    /// Multi-file only: matching files past the memory window not loaded in this call.
    pub files_not_scanned: usize,
    /// Content bytes of those unscanned files.
    pub bytes_not_scanned: u64,
    /// The `start_after_file` for the new request that scans past this partial result.
    pub next_start_after_file: Option<String>,
    /// Multi-file only: files the parser produced no syntax tree for.
    pub unparsed_files: Vec<String>,
    /// Multi-file only: matching files larger than `max_parse_bytes` (or the scan window).
    pub files_over_parse_limit: usize,
    /// Multi-file only: set when the execution time budget ran out while this file was parsed.
    pub parse_timed_out_file: Option<String>,
}
