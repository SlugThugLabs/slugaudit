//! Request and response schema types for the `leads` tool.

use super::super::mechanic::MechanicSource;
use super::super::page::{LegacyPaging, Page};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Longest `detail` string returned per row.
pub(in crate::tools) const MAX_DETAIL_CHARS: usize = 200;

/// Clips a row's `detail` to `MAX_DETAIL_CHARS` characters on a char boundary.
pub(in crate::tools) fn clip_detail(row: &mut LeadRow) {
    if row.detail.chars().count() <= MAX_DETAIL_CHARS {
        return;
    }
    row.detail = row.detail.chars().take(MAX_DETAIL_CHARS).collect();
    row.detail_clipped = true;
}

#[derive(Debug, Deserialize, JsonSchema)]
/// Which mechanical counts to run, optional pattern narrowing, the page to
/// read, and the row mode.
pub struct LeadsRequest {
    /// Any path inside the active project — required unless `cursor` is given.
    #[schemars(
        description = "Any path inside the active project, used to select the database; required unless `cursor` is given"
    )]
    #[serde(default)]
    pub path: Option<String>,
    /// Optional mechanic id; omit for counts only (`full`) or all rows (`compact`).
    #[schemars(description = "Optional mechanic id; omit for counts only")]
    #[serde(default)]
    pub mechanic: Option<String>,
    /// Optional path GLOB filter (substring match when it has no `*`/`?`).
    #[schemars(description = "Optional path GLOB filter (substring match when it has no `*`/`?`)")]
    #[serde(default)]
    pub pattern: Option<String>,
    /// Maximum rows in this page; omit for everything that fits `page_bytes`.
    #[schemars(
        description = "Max rows in this page; omit for everything that fits the byte budget"
    )]
    #[serde(default)]
    pub limit: Option<usize>,
    /// Previous page's `page.cursor`; omit for a fresh request.
    #[schemars(
        description = "The previous page's `page.cursor`, copied exactly; omit for a new request"
    )]
    #[serde(default)]
    pub cursor: Option<String>,
    /// Row representation: `full` (default) carries `detail`; `compact` drops it.
    #[schemars(description = "Row shape: `full` (default) carries `detail`; `compact` drops it")]
    #[serde(default)]
    pub mode: LeadMode,
    /// Page size in serialized JSON bytes for `rows`.
    #[schemars(
        description = "Page size in bytes of serialized rows; omit for the server default, clamped to the ceiling (reported)"
    )]
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// Removed `offset` / `expected_revision` parameters, accepted only to be refused.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

/// How a response renders its rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LeadMode {
    /// Each row carries its clipped `detail` text.
    #[default]
    Full,
    /// Each row is a [`CompactLeadRow`] triple with no `detail` text.
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// One mechanic's complete counts under the request's pattern.
pub struct MechanicCount {
    /// Stable mechanic id.
    pub mechanic: String,
    /// `session` or `builtin`.
    pub source: MechanicSource,
    /// Author's note for a session mechanic; `null` for a built-in.
    pub note: Option<String>,
    /// Total observations under `pattern`.
    pub count: i64,
    /// Distinct files containing at least one observation.
    pub files: i64,
    /// Sorted languages covered by this mechanic under `pattern`.
    pub covers: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One full-mode lead observation.
pub struct LeadRow {
    /// Project-relative file path.
    pub file: String,
    /// One-based start line, or `0` when the evidence carries no span.
    pub line: i64,
    /// Clipped observation text.
    pub detail: String,
    /// True when `detail` was clipped to `MAX_DETAIL_CHARS`.
    pub detail_clipped: bool,
}

/// One compact-mode lead triple.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompactLeadRow {
    /// Mechanic id.
    pub mechanic: String,
    /// Project-relative file path.
    pub file: String,
    /// One-based start line, or `0`.
    pub line: i64,
}

/// The rows of a page, shaped by `mode`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum LeadRows {
    /// Full rows carrying `detail`.
    Full(Vec<LeadRow>),
    /// Compact triples without `detail`.
    Compact(Vec<CompactLeadRow>),
}

/// One file's row count on this page.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FileCount {
    /// Project-relative path.
    pub file: String,
    /// Rows on this page belonging to `file`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One `leads` response: complete counts, paged rows, and coverage denominators.
pub struct LeadsResponse {
    /// Paging metadata for `rows`.
    pub page: Page,
    /// Per-mechanic counts (session entries for this project first, then built-ins).
    pub mechanic_counts: Vec<MechanicCount>,
    /// Untested-language notices for languages covered by this answer.
    pub notices: Vec<String>,
    /// Indexed code files under `pattern` whose grammar did not run or failed.
    pub files_not_parsed: i64,
    /// This page's rows.
    pub rows: LeadRows,
    /// Distinct files holding at least one observation in scope.
    pub files_total: i64,
    /// Distinct files covered through the end of this page.
    pub files_covered: i64,
    /// Per-file histogram for this page only.
    pub page_file_counts: Vec<FileCount>,
    /// True when row 0 alone exceeded `page.page_bytes`.
    pub oversized_row: bool,
}
