use super::context::{ensure_synced, guard_expected_revision, with_verified_read};
use super::page_size;
use super::query_clip::ClippedCell;
use super::query_exec::{RowTotal, run_query};
use super::query_value::NonFiniteCell;

use crate::error_codes::{self, ErrorCode};
use crate::model::ResourceLimits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_REVISION_BYTES: usize = 256;

#[derive(Debug, Deserialize, JsonSchema)]
/// A read-only SQL question over the project's own database, with stable
/// paging via `revision_id`/`expected_revision`.
pub struct QueryRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Read-only SQL against the project's own database — arbitrary joins,
    /// CTEs, and window functions are fine. Only SELECT is accepted: the
    /// statement is wrapped in a subquery, so writes and other statement
    /// forms (PRAGMA, EXPLAIN, VACUUM, ...) are rejected by the wrapper.
    /// There is no keyword blocklist or table allowlist, and the connection
    /// itself is read-only as a second boundary.
    pub sql: String,
    /// Number of matching rows to skip when paging through a result.
    /// Combined with the response's `next_offset`, and valid only while
    /// the revision id stays the same across pages (see [`QueryResponse`]).
    #[serde(default)]
    pub offset: usize,
    /// Set to the previous page's `revision_id` when resuming a sweep. If
    /// the index changed since that page, the call is refused with the
    /// `stale_revision` code instead of returning a page whose ordering no
    /// longer matches the offset: `files`/`evidence` hold only the current
    /// revision, so a stored offset means nothing once the tree changes.
    #[serde(default)]
    pub expected_revision: Option<String>,
    /// Page size in serialized JSON bytes. Omit it for the server default
    /// (64 KiB unless configured otherwise; the response's `page_bytes` says
    /// what applied). A larger value returns a larger page, up to the hard
    /// ceiling `max_query_response_bytes`: a value above the ceiling is
    /// clamped and the response reports `page_bytes_clamped_from`. Pages are
    /// cut on a row boundary; when the first row alone is larger than the page,
    /// its TEXT/BLOB cells are clipped to fit and listed in `clipped_cells`
    /// with the `substr` expression that continues each one. Raise
    /// `page_bytes` to avoid clipping.
    #[serde(default)]
    pub page_bytes: Option<usize>,
}

/// Arbitrary read-only query results: one JSON object per row, column
/// names as keys. This is the general-purpose tool — search, symbol/
/// import/diagnostic lookup, dependency traversal, and source retrieval
/// all reach through it as ordinary `SELECT`s against the schema.
///
/// **Paging contract**: rows are returned in the query's own row order —
/// no `ORDER BY` is added, so pagination is stable only while the
/// underlying revision does not change. When a concurrent publish lands
/// between pages, row order can shift and `next_offset` paging may skip
/// or duplicate rows; the AI detects that boundary by comparing the
/// `revision_id` on each page and should restart paging (or accept the
/// new revision's ordering) when it changes. Passing the previous page's
/// `revision_id` as `expected_revision` makes the server enforce that: the
/// resumed page is refused with the `stale_revision` code instead of
/// returning rows from a different ordering.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    /// Revision id the page was read from. Compare across pages to detect
    /// a revision change that invalidates `next_offset` paging, or pass it
    /// back as `expected_revision` to have the change refused.
    pub revision_id: String,
    pub rows: Vec<serde_json::Value>,
    pub truncated: bool,
    /// Use this as the next request's `offset` when present. Only valid
    /// while `revision_id` is unchanged (see the paging contract above).
    pub next_offset: Option<usize>,
    /// Exact row count of the whole result, ignoring `offset` and the page
    /// cut, computed by SQLite over the same statement. Null only when
    /// counting exhausted its budget; `total_rows_unavailable` then says why.
    /// Never an estimate.
    pub total_rows: Option<i64>,
    /// Why `total_rows` is null (the budget that stopped the count); null
    /// whenever `total_rows` is present.
    pub total_rows_unavailable: Option<String>,
    /// The page size that applied to this call, in serialized JSON bytes: the
    /// request's `page_bytes` or the server default, clamped to the ceiling.
    pub page_bytes: usize,
    /// Non-null when the requested (or configured default) page size exceeded
    /// `max_query_response_bytes` and was clamped: the size that was asked
    /// for. `page_bytes` is then the ceiling.
    pub page_bytes_clamped_from: Option<usize>,
    /// Serialized JSON bytes of the returned `rows`, measured per row.
    pub rows_bytes: usize,
    /// The TEXT/BLOB cells cut from the first row because the row alone was
    /// larger than `page_bytes`; empty when nothing was cut. Each entry gives
    /// the absolute `row` index, `column`, `returned` and `total` length in
    /// `substr` units (characters for text, bytes for a blob), `next_start`
    /// (1-based) and a ready `continue_with` expression. No data is lost: the
    /// rest of every clipped cell is one `substr` away. If this is empty,
    /// every cell is complete. Only a row whose non-text cells alone exceed
    /// the page (or a page smaller than one character per cell) can leave
    /// `rows_bytes` above `page_bytes`.
    pub clipped_cells: Vec<ClippedCell>,
    /// REAL cells that are `null` in `rows` only because JSON cannot carry an
    /// infinity: each entry gives the absolute `row`, the `column` and the real
    /// `value` (`"Infinity"`, `"-Infinity"` or `"NaN"`). A `null` that is not
    /// listed here is SQL NULL. Empty when every REAL is finite.
    pub non_finite_cells: Vec<NonFiniteCell>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `sql` is
/// empty or too long, the query fails to parse or execute (including a
/// write or any other non-SELECT statement, which the wrapper refuses),
/// the VM-step or wall-clock budget is exhausted, or a result value can't be
/// represented (including a single TEXT/BLOB value over the per-value cap).
pub fn query(
    request: &Parameters<QueryRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    let limits = super::resolve_limits(&request.0.path)?;
    query_with_limits(request, &limits, sink, manager)
}

/// Test-only seam: production code always goes through [`query`] with
/// `process_limits`; tests inject tighter limits to exercise
/// truncation and budget paths without waiting out production-sized caps.
/// Private (not `pub`), but visible to the `tests` submodule below like any
/// other item in this module.
fn query_with_limits(
    request: &Parameters<QueryRequest>,
    limits: &ResourceLimits,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    let QueryRequest {
        path,
        sql,
        offset,
        expected_revision,
        page_bytes,
    } = &request.0;
    if crate::util::exceeds_input_bytes(path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    if expected_revision
        .as_deref()
        .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_REVISION_BYTES))
    {
        return Err(ErrorData::invalid_params(
            "expected_revision exceeds 256 bytes",
            None,
        ));
    }
    let trimmed = sql.trim().trim_end_matches(';');
    if trimmed.is_empty() {
        return Err(ErrorData::invalid_params("sql must not be empty", None));
    }
    if sql.len() > limits.max_query_sql_bytes {
        return Err(ErrorData::invalid_params(
            format!("sql exceeds {} bytes", limits.max_query_sql_bytes),
            error_codes::payload(ErrorCode::SqlTooLarge),
        ));
    }
    // SQLite stores an integer literal too large for i64 as a REAL, so an
    // `OFFSET` past i64::MAX silently becomes a floating-point offset and pages
    // from an unpredictable position instead of failing. Refuse it here as a
    // contract error, before the wrapper is built. No error code: this is
    // one-off validation prose a caller cannot act on beyond fixing its own
    // arithmetic (see the deliberate list in `error_codes`).
    if *offset as u64 > i64::MAX as u64 {
        return Err(ErrorData::invalid_params(
            format!(
                "offset {offset} exceeds the maximum supported page offset ({}); \
                 page from the start or from a returned next_offset",
                i64::MAX
            ),
            None,
        ));
    }

    let page = page_size::resolve(*page_bytes, limits)?;

    let synced = ensure_synced(path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, expected_revision.as_deref())?;
    let revision_id = synced.revision_id.clone();
    let collected = with_verified_read(&synced, |tx| {
        run_query(tx, trimmed, *offset, page.bytes, limits)
    })?;
    let rows = collected.rows;
    let truncated = collected.truncated;
    let next_offset = truncated.then_some(offset.saturating_add(rows.len()));
    let (total_rows, total_rows_unavailable) = match collected.total {
        RowTotal::Exact(total) => (Some(total), None),
        RowTotal::Unavailable(reason) => (None, Some(reason.to_owned())),
    };

    // Row count and truncation, never the SQL text or the rows themselves.
    tracing::info!(
        revision_id,
        row_count = rows.len(),
        truncated,
        total_rows,
        clipped_cells = collected.clipped.len(),
        non_finite_cells = collected.non_finite.len(),
        page_bytes = page.bytes,
        clamped = page.clamped_from.is_some(),
        "query executed"
    );
    Ok(Json(QueryResponse {
        revision_id,
        rows,
        truncated,
        next_offset,
        total_rows,
        total_rows_unavailable,
        page_bytes: page.bytes,
        page_bytes_clamped_from: page.clamped_from,
        rows_bytes: collected.rows_bytes,
        clipped_cells: collected.clipped,
        non_finite_cells: collected.non_finite,
    }))
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_safety_tests.rs"]
mod safety_tests;
