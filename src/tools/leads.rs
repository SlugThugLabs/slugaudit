//! `leads` — unranked mechanical observations about an indexed project.
//!
//! This tool surfaces facts the index already holds — call sites by callee
//! name, diagnostics, comment markers, credential-shaped file paths — and
//! deliberately ranks, scores, orders, and interprets none of them. Which of
//! these is worth pursuing is the calling agent's judgment; see
//! `INSTRUCTIONS` in `src/server.rs` for that contract in the agent's own
//! words.
//!
//! Every row is derived at read time from `evidence` / `files`; nothing is
//! persisted, so there is no risk table and no second source of truth.
//!
//! # Paging
//!
//! A page is cut by the caller's `limit` and by the page byte budget (the
//! request's `page_bytes` or `ResourceLimits::default_page_bytes`, clamped to
//! `max_query_response_bytes`, the same page size `query` spends), never by a
//! row ceiling: the only row cap was a copy of `query`'s and it made the tool that exists for accuracy the one that truncated
//! silently. Every rows response therefore carries `returned` / `total` /
//! `complete`, the file denominators (`files_total` / `files_covered` — at
//! scale the number an agent can actually exhaust is *files containing at
//! least one observation*, not rows), and `limited_by`, which names what
//! stopped the page.

use super::context::{ensure_synced, guard_expected_revision, with_verified_read};
use super::leads_mechanics::resolve_mechanic;
use super::leads_page::{self, Paging};
use crate::model::ResourceLimits;
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(test)]
pub(super) use super::leads_query::{MAX_DETAIL_CHARS, clip_detail};
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILTER_BYTES: usize = 8 * 1024;
const MAX_REVISION_BYTES: usize = 256;

#[derive(Debug, Deserialize, JsonSchema)]
/// Which mechanical counts to run, optional pattern narrowing, the page to
/// read, and the row mode.
pub struct LeadsRequest {
    /// Any path inside the active project — selects which project's database
    /// to read. Must be a path inside an enabled project.
    #[schemars(description = "Any path inside the active project, used to select the database")]
    pub path: String,
    /// Omit to get `mechanic_counts` only. Pass one of the ids from that
    /// response to also get rows for that mechanic.
    #[schemars(description = "Optional mechanic id; omit for counts only")]
    #[serde(default)]
    pub mechanic: Option<String>,
    /// Path filter over the project-relative file path, compiled the same
    /// way `structure`'s `pattern` is: a GLOB, where a pattern containing no
    /// `*` or `?` is treated as a substring match. `src/auth/*` narrows a
    /// mechanic to one directory; `tests` narrows it to anything with
    /// `tests` in the path. It filters the counts and the rows alike.
    #[schemars(description = "Optional path GLOB filter (substring match when it has no `*`/`?`)")]
    #[serde(default)]
    pub pattern: Option<String>,
    /// Maximum rows in this page. Omitted means every row that fits the
    /// response byte budget — the default is everything, not a token-thrift
    /// sample. `0` is treated as `1`: a zero-row page could not make
    /// progress and `next_offset` would repeat it forever.
    #[schemars(
        description = "Max rows in this page; omit for everything that fits the byte budget"
    )]
    #[serde(default)]
    pub limit: Option<usize>,
    /// Rows to skip, combined with the response's `next_offset`.
    #[schemars(description = "Rows to skip when paging")]
    #[serde(default)]
    pub offset: Option<usize>,
    /// Set to the previous page's `revision_id` when resuming a sweep. If
    /// the index has changed since that page, the call is refused with the
    /// `stale_revision` code instead of silently skipping or repeating rows:
    /// the index keeps no per-revision copy of `files` / `evidence`, so a
    /// stored offset only means anything against the revision it came from.
    #[schemars(description = "Pin the revision a resumed page expects; a mismatch is refused")]
    #[serde(default)]
    pub expected_revision: Option<String>,
    /// Row representation. `full` (the default) returns each row's `detail`
    /// text. `compact` returns `mechanic` / `file` / `line` only — no
    /// per-row text — so the mechanical surface costs a fraction of the
    /// bytes; with no `mechanic` it enumerates every mechanic's rows in one
    /// response instead of returning counts only.
    #[schemars(description = "Row shape: `full` (default) carries `detail`; `compact` drops it")]
    #[serde(default)]
    pub mode: LeadMode,
    /// Page size in serialized JSON bytes for `rows`. Omit it for the server
    /// default (64 KiB unless configured otherwise; the response's
    /// `row_budget_bytes` says what applied). A larger value returns a larger
    /// page, up to the hard ceiling `max_query_response_bytes`: a value above
    /// the ceiling is clamped and the response reports
    /// `page_bytes_clamped_from`. A single row larger than the page is still
    /// returned and flagged with `oversized_row`.
    #[schemars(
        description = "Page size in bytes of serialized rows; omit for the server default, clamped to the ceiling (reported)"
    )]
    #[serde(default)]
    pub page_bytes: Option<usize>,
}

/// How a response renders its rows. See [`LeadsResponse::rows`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LeadMode {
    /// Each row carries its `detail` text (clipped; see [`LeadRow`]).
    #[default]
    Full,
    /// Each row is a [`CompactLeadRow`]: no per-row text at all.
    Compact,
}

/// Which languages a mechanic was actually measured over. A zero count on a
/// [`MechanicCoverage::RustOnly`] mechanic means *unchecked*, not *absent* —
/// without this field that distinction is only prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MechanicCoverage {
    /// Only the Rust call walker produces the evidence this mechanic reads.
    RustOnly,
    /// Produced for every indexed language.
    AllLanguages,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One mechanic's complete counts under the request's pattern — complete
/// regardless of paging, with no ordering by importance.
pub struct MechanicCount {
    /// Stable id — pass it back as `mechanic` to get this mechanic's rows.
    pub mechanic: String,
    /// How many observations this mechanic has in the current revision
    /// under the same `pattern`. Complete, never paged.
    pub count: i64,
    /// Distinct files containing at least one observation. This is the
    /// denominator to enumerate: a 50,000-row mechanic is usually far fewer
    /// files, and a file is the unit a sweep can finish.
    pub files: i64,
    /// The languages this count covers. `rust_only` with a zero count on a
    /// non-Rust project means nothing was checked.
    pub covers: MechanicCoverage,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One lead observation: file, line (0 when the evidence carries no span),
/// and the mechanic's own clipped detail text.
pub struct LeadRow {
    /// Project-relative path of the file the observation is in.
    pub file: String,
    /// One-based start line, or `0` when the evidence carries no span.
    pub line: i64,
    /// The mechanic's own text: callee text, diagnostic message, or comment
    /// text. Empty for path-shaped mechanics. Clipped to `MAX_DETAIL_CHARS`
    /// characters when longer — check `detail_clipped`.
    pub detail: String,
    /// True when `detail` was clipped to `MAX_DETAIL_CHARS`; the text is then
    /// a prefix of the stored evidence and the remainder exists only in the
    /// file itself. False means the whole stored text is present.
    pub detail_clipped: bool,
}

/// One row of a compact page: which mechanic observed what, at a path and
/// line. No text is carried — read the file, or ask for `mode: full` — which
/// is what makes a whole-surface sweep affordable.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompactLeadRow {
    /// The mechanic this row belongs to — an id from `mechanic_counts`, and
    /// the id to pass as `mechanic` for that mechanic's `detail` text.
    pub mechanic: String,
    /// Project-relative path of the file the observation is in.
    pub file: String,
    /// One-based start line, or `0` when the evidence carries no span.
    pub line: i64,
}

/// The rows of a page, shaped by the request's `mode`; one response carries
/// one variant, never a mix. `full` (the default) is [`LeadRow`] with its
/// `detail` text; `compact` is [`CompactLeadRow`] with no per-row text. All
/// the envelope's completeness fields mean the same thing either way.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum LeadRows {
    Full(Vec<LeadRow>),
    Compact(Vec<CompactLeadRow>),
}

/// One file's share of a page, in first-appearance order.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FileCount {
    /// Project-relative path.
    pub file: String,
    /// Rows of this mechanic in *this page* that belong to this file — not
    /// the file's total, which is what `files_total` / `files_covered`
    /// track across pages.
    pub count: i64,
}

/// What stopped a page. Reports the cap that acted; never a judgment about
/// whether the page was the right size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageLimit {
    /// The mechanic was exhausted: `complete` is true.
    None,
    /// The caller's `limit` was reached; more rows exist.
    Limit,
    /// The response byte budget was reached; more rows exist. A
    /// byte-limited page is by construction at or below any `limit` that
    /// was set, so raising `limit` returns nothing more: raise `page_bytes`
    /// to enlarge the page (up to the server ceiling), or page on.
    Bytes,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One leads call: complete counts, the page's rows, and the paging/budget
/// truth that makes them quotable as-is.
pub struct LeadsResponse {
    /// Revision the counts and rows were read from. Pass it as
    /// `expected_revision` on the next page to have a changed index refused
    /// rather than mis-paged.
    pub revision_id: String,
    /// Every mechanic this revision can answer under the request's
    /// `pattern`, with its row and distinct-file counts and what it covers.
    /// Counts are complete regardless of `limit`/`offset`, and carry no
    /// ordering by importance — the order is the fixed declaration order.
    pub mechanic_counts: Vec<MechanicCount>,
    /// The page's rows, shaped by the request's `mode`. Empty when a `full`
    /// call supplied no `mechanic`; a `compact` call with no `mechanic`
    /// enumerates every mechanic instead, one triple per row.
    pub rows: LeadRows,
    /// Rows in this page.
    pub returned: usize,
    /// Total rows in the enumerated scope — the requested mechanic, or (in
    /// `compact` mode with no `mechanic`) every mechanic — under the same
    /// `pattern`; `0` when a `full` call requested no mechanic.
    pub total: i64,
    /// True when this page reaches the end of the scope
    /// (`offset + returned >= total`), and when a `full` call requested no
    /// mechanic. False means rows remain and `next_offset` is present.
    pub complete: bool,
    /// Distinct files holding at least one observation of any mechanic in
    /// scope — the denominator a sweep can exhaust. `0` when a `full` call
    /// requested no mechanic. For a single mechanic it equals that
    /// `mechanic_counts` entry's `files`.
    pub files_total: i64,
    /// Distinct files represented in rows `0..offset + returned` of the
    /// scope. Cumulative for a sweep at one revision: rows are read in a
    /// fixed order — within a single mechanic, by path then line; for a
    /// compact call with no mechanic, by mechanic declaration order then
    /// path then line — so a file's rows for one mechanic are contiguous,
    /// but a file with rows under several mechanics appears in several
    /// groups and can span several compact pages. It is still counted once.
    /// `files_total - files_covered` is what is left.
    pub files_covered: i64,
    /// Rows grouped per file for this page only.
    pub page_file_counts: Vec<FileCount>,
    /// What stopped the page: the scope ended, the caller's `limit`, or the
    /// byte budget.
    pub limited_by: PageLimit,
    /// Serialized-JSON byte budget applied to `rows` this call: the
    /// request's `page_bytes` or the server default
    /// (`SLUGAUDIT_DEFAULT_PAGE_BYTES`), clamped to the ceiling
    /// `max_query_response_bytes`, the same page size `query` spends. One row
    /// is always returned even if it alone exceeds it (see `oversized_row`).
    pub row_budget_bytes: usize,
    /// Non-null when the requested (or configured default) page size exceeded
    /// `max_query_response_bytes` and was clamped: the size that was asked
    /// for. `row_budget_bytes` is then the ceiling.
    pub page_bytes_clamped_from: Option<usize>,
    /// True when `rows_bytes` exceeds `row_budget_bytes` because the first
    /// row alone is larger than the page. It is returned whole so paging
    /// makes progress and nothing is lost.
    pub oversized_row: bool,
    /// Serialized-JSON bytes of the returned `rows`, measured per row
    /// exactly as `query` measures its values. The rest of this envelope
    /// (`mechanic_counts` — at most one entry per mechanic — and
    /// `page_file_counts`) is additional and small relative to the rows.
    pub rows_bytes: usize,
    /// Pass as `offset` on the next call while `complete` is false, with
    /// `expected_revision` set to this response's `revision_id`.
    pub next_offset: Option<usize>,
}

/// Unranked mechanical observations from the project's current revision.
///
/// # Errors
///
/// Returns an error if `request.path` is not inside an active project, if
/// syncing or reading the database fails, if the requested `mechanic` id is
/// not one this tool defines, if `expected_revision` no longer matches the
/// current revision (a resumed page — `stale_revision`), or if a read races
/// a concurrent publish (the caller retries).
pub fn leads(
    request: &Parameters<LeadsRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<LeadsResponse>, ErrorData> {
    let limits = super::resolve_limits(&request.0.path)?;
    leads_with_limits(request, &limits, sink, manager)
}

/// Test-only seam: production code always goes through [`leads`] with the
/// process/project limits; tests inject a tighter `max_query_response_bytes`
/// to exercise the byte budget without generating 32 MiB of rows.
fn leads_with_limits(
    request: &Parameters<LeadsRequest>,
    limits: &ResourceLimits,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<LeadsResponse>, ErrorData> {
    validate_leads_request(
        &request.0.path,
        request.0.mechanic.as_deref(),
        request.0.pattern.as_deref(),
        request.0.expected_revision.as_deref(),
    )?;
    let page = super::page_size::resolve(request.0.page_bytes, limits)?;
    let synced = ensure_synced(&request.0.path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, request.0.expected_revision.as_deref())?;
    let selected = resolve_mechanic(request.0.mechanic.as_deref())?;
    let paging = Paging {
        limit: request.0.limit.map(|rows| rows.max(1)),
        offset: request.0.offset.unwrap_or(0),
        budget_bytes: page.bytes,
        clamped_from: page.clamped_from,
    };
    let response = with_verified_read(&synced, |tx| {
        leads_page::build(
            tx,
            &synced.revision_id,
            selected,
            request.0.mode,
            request.0.pattern.as_deref(),
            paging,
        )
    })?;

    tracing::info!(
        revision_id = response.0.revision_id,
        row_count = response.0.returned,
        total = response.0.total,
        complete = response.0.complete,
        "leads built"
    );
    Ok(response)
}

/// Validates the byte caps on the request's free-form fields before any
/// database work. Error text is part of the tool contract.
fn validate_leads_request(
    path: &str,
    mechanic: Option<&str>,
    pattern: Option<&str>,
    expected_revision: Option<&str>,
) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    for (name, value) in [("mechanic", mechanic), ("pattern", pattern)] {
        if value.is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_FILTER_BYTES)) {
            return Err(ErrorData::invalid_params(
                format!("{name} exceeds 8192 bytes"),
                None,
            ));
        }
    }
    if expected_revision
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_REVISION_BYTES))
    {
        return Err(ErrorData::invalid_params(
            "expected_revision exceeds 256 bytes",
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "leads_tests.rs"]
mod tests;
