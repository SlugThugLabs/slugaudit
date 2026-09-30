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
//! A page is cut by the caller's `limit` and by the response byte budget
//! (`ResourceLimits::max_query_response_bytes`, the same budget `query`
//! spends), never by a row ceiling: the only row cap was a copy of `query`'s
//! and it made the tool that exists for accuracy the one that truncated
//! silently. Every rows response therefore carries `returned` / `total` /
//! `complete`, the file denominators (`files_total` / `files_covered` — at
//! scale the number an agent can actually exhaust is *files containing at
//! least one observation*, not rows), and `limited_by`, which names what
//! stopped the page.

use super::context::{ensure_synced, guard_expected_revision, with_verified_read};
use super::leads_mechanics::resolve_mechanic;
use super::leads_query::{count_all, files_covered, page_file_counts, rows_for};
use crate::model::{ResourceLimits, process_limits};
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Longest `detail` string returned per row. Callee text and comment lines
/// can be arbitrarily long; the caller can always read the file itself.
const MAX_DETAIL_CHARS: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
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
    /// The response byte budget was reached; more rows exist. Raise
    /// `limit` if one was set, or page on.
    Bytes,
}

#[derive(Debug, Serialize, JsonSchema)]
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
    /// Empty when the request supplied no `mechanic`.
    pub rows: Vec<LeadRow>,
    /// Rows in this page.
    pub returned: usize,
    /// Total rows for the requested mechanic under the same `pattern`; `0`
    /// when no mechanic was requested.
    pub total: i64,
    /// True when this page reaches the end of the mechanic
    /// (`offset + returned >= total`), and when no mechanic was requested.
    /// False means rows remain and `next_offset` is present.
    pub complete: bool,
    /// Distinct files holding at least one observation of the requested
    /// mechanic — the denominator a sweep can exhaust. `0` when no mechanic
    /// was requested. Equal to the selected `mechanic_counts` entry's
    /// `files`.
    pub files_total: i64,
    /// Distinct files represented in rows `0..offset + returned` of the
    /// mechanic. Cumulative for a sweep at one revision, because rows are
    /// ordered by path and a page boundary can split a file but never
    /// reorder the files. `files_total - files_covered` is what is left.
    pub files_covered: i64,
    /// Rows grouped per file for this page only.
    pub page_file_counts: Vec<FileCount>,
    /// What stopped the page: the mechanic ended, the caller's `limit`, or
    /// the byte budget.
    pub limited_by: PageLimit,
    /// Serialized-JSON byte ceiling applied to `rows` this call — the value
    /// of `max_query_response_bytes`
    /// (`SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES`), the same budget `query`
    /// spends. One row is always returned even if it alone exceeds it.
    pub row_budget_bytes: usize,
    /// Serialized-JSON bytes of the returned `rows`, measured per row
    /// exactly as `query` measures its values. The rest of this envelope
    /// (`mechanic_counts` — at most one entry per mechanic — and
    /// `page_file_counts`) is additional and small relative to the rows.
    pub rows_bytes: usize,
    /// Pass as `offset` on the next call while `complete` is false, with
    /// `expected_revision` set to this response's `revision_id`.
    pub next_offset: Option<usize>,
}

/// Clips a row's `detail` to `MAX_DETAIL_CHARS` characters on a char
/// boundary, setting `detail_clipped` only when characters were removed.
/// No shared truncation helper exists in `crate::util`.
pub(super) fn clip_detail(row: &mut LeadRow) {
    if row.detail.chars().count() <= MAX_DETAIL_CHARS {
        return;
    }
    row.detail = row.detail.chars().take(MAX_DETAIL_CHARS).collect();
    row.detail_clipped = true;
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
    let limits = resolve_leads_limits(&request.0.path);
    leads_with_limits(request, &limits, sink, manager)
}

fn resolve_leads_limits(path: &str) -> ResourceLimits {
    match crate::project::find_project_root(std::path::Path::new(path)) {
        Ok(root) => crate::project::ProjectConfig::load_or_default(root.as_path())
            .apply_to_limits(*process_limits()),
        Err(_) => *process_limits(),
    }
}

/// Test-only seam: production code always goes through [`leads`] with
/// [`process_limits`]; tests inject a tighter `max_query_response_bytes` to
/// exercise the byte budget without generating 32 MiB of rows.
fn leads_with_limits(
    request: &Parameters<LeadsRequest>,
    limits: &ResourceLimits,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<LeadsResponse>, ErrorData> {
    let LeadsRequest {
        path,
        mechanic,
        pattern,
        limit,
        offset,
        expected_revision,
    } = &request.0;
    let synced = ensure_synced(path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, expected_revision.as_deref())?;
    let selected = resolve_mechanic(mechanic.as_deref())?;
    let offset = offset.unwrap_or(0);
    let limit = limit.map(|rows| rows.max(1));
    let pattern = pattern.as_deref();
    let budget_bytes = limits.max_query_response_bytes;

    let response = with_verified_read(&synced, |tx| {
        let mechanic_counts = count_all(tx, pattern)?;
        let Some(mechanic) = selected else {
            return Ok(Json(LeadsResponse {
                revision_id: synced.revision_id.clone(),
                mechanic_counts,
                rows: Vec::new(),
                returned: 0,
                total: 0,
                complete: true,
                files_total: 0,
                files_covered: 0,
                page_file_counts: Vec::new(),
                limited_by: PageLimit::None,
                row_budget_bytes: budget_bytes,
                rows_bytes: 0,
                next_offset: None,
            }));
        };
        let entry = mechanic_counts
            .iter()
            .find(|entry| entry.mechanic == mechanic.id);
        let total = entry.map_or(0, |entry| entry.count);
        let files_total = entry.map_or(0, |entry| entry.files);
        let page = rows_for(tx, mechanic, pattern, limit, offset, budget_bytes)?;
        let returned = page.rows.len();
        let through = offset.saturating_add(returned);
        let seen = i64::try_from(through).unwrap_or(i64::MAX);
        let complete = seen >= total;
        // The page that ends the mechanic has seen every file by
        // construction; earlier pages ask the index how many distinct files
        // their rows reach.
        let covered = if complete {
            files_total
        } else {
            files_covered(tx, mechanic, pattern, through)?
        };
        let counts = page_file_counts(&page.rows);
        Ok(Json(LeadsResponse {
            revision_id: synced.revision_id.clone(),
            mechanic_counts,
            rows: page.rows,
            returned,
            total,
            complete,
            files_total,
            files_covered: covered,
            page_file_counts: counts,
            limited_by: page.limited_by,
            row_budget_bytes: budget_bytes,
            rows_bytes: page.bytes,
            next_offset: (!complete).then_some(through),
        }))
    })?;

    tracing::info!(
        revision_id = response.0.revision_id,
        mechanic = ?request.0.mechanic,
        row_count = response.0.returned,
        total = response.0.total,
        complete = response.0.complete,
        "leads built"
    );
    Ok(response)
}

#[cfg(test)]
#[path = "leads_tests.rs"]
mod tests;
