use super::page::{LegacyPaging, Page, PageLimit};
use super::page_size::{self, PageSize};
use super::paginate::page_request;
use super::query_clip::ClippedCell;
use super::query_exec::run_query;
pub(super) use super::query_sql::{
    ABORT_NONE, ABORT_STEP_BUDGET, ABORT_WALL_CLOCK, describe_error, refuse_wrapped,
    single_statement_body,
};
use super::query_value::NonFiniteCell;

use crate::error_codes::{self, ErrorCode};
use crate::model::ResourceLimits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::MAX_PATH_BYTES;

#[derive(Debug, Deserialize, JsonSchema)]
/// A read-only SQL question over the project's own database, paged with
/// `cursor`.
pub struct QueryRequest {
    /// Any path inside the active project. Required unless `cursor` is given.
    #[serde(default)]
    pub path: Option<String>,
    /// Read-only SQL against the project's own database — arbitrary joins,
    /// CTEs, and window functions are fine. Exactly one SELECT or WITH
    /// statement is accepted. A PRAGMA is refused before anything is
    /// prepared, wherever it sits in the input. The statement is then wrapped
    /// as a subquery, which refuses every other kind (a write, EXPLAIN,
    /// VACUUM, ATTACH, DDL). There is no keyword blocklist or table
    /// allowlist. The schema-introspection table functions
    /// (`pragma_table_info`, `pragma_table_xinfo`, `pragma_table_list`,
    /// `pragma_index_list`, `pragma_index_info`, `pragma_index_xinfo`,
    /// `pragma_foreign_key_list`) are ordinary SELECTs and work.
    /// Required unless `cursor` is given; ignored with a `cursor`.
    #[serde(default)]
    pub sql: String,
    /// The previous page's `page.cursor`, copied exactly. It continues the
    /// stored result of the call that returned it, so `path` and `sql` are
    /// not needed (and are ignored). Omit it to run a new query.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size in serialized JSON bytes. Omit it for the server default
    /// (64 KiB unless configured otherwise; the response's `page.page_bytes`
    /// says what applied). A larger value returns a larger page, up to the
    /// hard ceiling `max_query_response_bytes`: a value above the ceiling is
    /// clamped, and a page the ceiling cut reports `page.limited_by` =
    /// `ceiling`. Pages are
    /// cut on a row boundary; when the first row alone is larger than the page,
    /// its TEXT/BLOB cells are clipped to fit and listed in `clipped_cells`
    /// with the `substr` expression that continues each one. Raise
    /// `page_bytes` to avoid clipping. May change between pages.
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// The `offset` and `expected_revision` parameters, accepted only to refuse
    /// them naming `cursor`.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

/// Arbitrary read-only query results: one JSON object per row, column
/// names as keys. This is the general-purpose tool — search, symbol/
/// import/diagnostic lookup, dependency traversal, and source retrieval
/// all reach through it as ordinary `SELECT`s against the schema.
///
/// **Paging contract**: the whole result is computed once, when the query
/// runs without a `cursor`, and stored; rows are in the query's own row order
/// (no `ORDER BY` is added). While `page.cursor` is present, call again with
/// only that `cursor` for the next page: every page comes from the stored
/// copy, so later edits to the project never move, skip or repeat a row, and
/// every page reports the `revision_id` of the first. A query that exceeds its
/// VM-step or wall-clock budget fails as a whole and returns no cursor.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    /// Paging for `rows`, cut by bytes on a row boundary. `total` is the exact
    /// row count of the whole stored result.
    pub page: Page,
    /// This page's rows, one JSON object per row with column names as keys,
    /// in the query's own row order.
    pub rows: Vec<serde_json::Value>,
    /// The TEXT/BLOB cells cut from the first row because the row alone was
    /// larger than `page_bytes`; empty when nothing was cut. Each entry gives
    /// the absolute `row` index, `column`, `returned` and `total` length in
    /// `substr` units (characters for text, bytes for a blob), `next_start`
    /// (1-based) and a ready `continue_with` expression. No data is lost: the
    /// rest of every clipped cell is one `substr` away. If this is empty,
    /// every cell is complete. Only a row whose non-text cells alone exceed
    /// the page (or a page smaller than one character per cell) can leave
    /// `page.bytes_used` above `page.page_bytes`.
    pub clipped_cells: Vec<ClippedCell>,
    /// REAL cells that are `null` in `rows` only because JSON cannot carry an
    /// infinity: each entry gives the absolute `row`, the `column` and the real
    /// `value` (`"Infinity"`, `"-Infinity"` or `"NaN"`). A `null` that is not
    /// listed here is SQL NULL. Empty when every REAL is finite.
    pub non_finite_cells: Vec<NonFiniteCell>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project (or neither it
/// nor a `cursor` is given), `sql` is empty or too long, the query fails to
/// parse or execute (including a write or any other non-SELECT statement,
/// which the wrapper refuses), the VM-step or wall-clock budget is exhausted
/// (the whole call fails and nothing is stored), the result's column names
/// repeat (the caller must alias them), a result value can't be represented
/// (including a single TEXT/BLOB value over the per-value cap), the request
/// carries `offset`/`expected_revision`, or the cursor is unknown
/// or expired.
pub fn query(
    request: &Parameters<QueryRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    // Reject legacy paging and oversized paths before walking the filesystem
    // to resolve project limits, matching `leads`, `structure`, and
    // `finding_read`.
    request.0.legacy_paging.refuse()?;
    validate_entry(&request.0)?;
    // A continuation names no project and reads no limit: its page budget and
    // the ceiling an explicit `page_bytes` is clamped to are those stored with
    // the result, so the deterministic default only fills the unused argument.
    // A new request runs under its project's. Without either, `page_request`
    // refuses the call below.
    let limits = match (&request.0.cursor, &request.0.path) {
        (None, Some(path)) => super::resolve_limits(path)?,
        _ => ResourceLimits::default(),
    };
    query_with_limits(request, &limits, sink, manager)
}

/// [`query`] with the limits already resolved; tests inject tighter limits to
/// exercise truncation and budget paths without waiting out production-sized
/// caps. A continuation reads no limit but the ones stored with its result.
fn query_with_limits(
    request: &Parameters<QueryRequest>,
    limits: &ResourceLimits,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    let QueryRequest {
        path,
        cursor,
        page_bytes,
        legacy_paging,
        ..
    } = &request.0;
    legacy_paging.refuse()?;
    let trimmed = validate_query_request(request, limits)?;
    let paging = page_request(cursor.as_deref(), path.as_deref(), sink)?;
    let page = page_size::resolve(*page_bytes, limits)?;
    let continuation = cursor.is_some();

    // Only a budget the request asked for overrides the one page one used.
    let (collected, stored_page, at_ceiling) =
        run_query(manager, paging, trimmed, limits, page, *page_bytes)?;
    let complete = stored_page.complete;
    // The budget that applied (a continuation may inherit page one's) cuts at
    // the ceiling exactly when it is the ceiling.
    let applied = PageSize {
        bytes: stored_page.page_bytes,
        at_ceiling,
    };
    let limited_by = if complete {
        PageLimit::None
    } else {
        applied.byte_cut()
    };

    // Row count and the page cut, never the SQL text or the rows themselves.
    tracing::info!(
        revision_id = stored_page.revision_id,
        row_count = collected.rows.len(),
        complete,
        total = stored_page.total,
        continuation,
        limited_by = ?limited_by,
        clipped_cells = collected.clipped.len(),
        non_finite_cells = collected.non_finite.len(),
        page_bytes = stored_page.page_bytes,
        "query executed"
    );
    Ok(Json(QueryResponse {
        page: Page {
            returned: collected.rows.len(),
            bytes_used: collected.bytes_used,
            limited_by,
            ..stored_page
        },
        rows: collected.rows,
        clipped_cells: collected.clipped,
        non_finite_cells: collected.non_finite,
    }))
}

/// Rejects an oversized `path` on a fresh request before any filesystem or
/// limit-resolution work.
fn validate_entry(request: &QueryRequest) -> Result<(), ErrorData> {
    if request.cursor.is_none()
        && request
            .path
            .as_deref()
            .is_some_and(|path| crate::util::exceeds_input_bytes(path, MAX_PATH_BYTES))
    {
        return Err(ErrorData::invalid_params(
            format!("path exceeds {MAX_PATH_BYTES} bytes"),
            None,
        ));
    }
    Ok(())
}

/// Rejects oversized or empty inputs before any sync or SQL work, in the
/// order the tool contract documents, and returns the SQL text without
/// surrounding whitespace or terminating `;`. A continuation carries nothing
/// else to check: its `path` and `sql` are ignored.
fn validate_query_request<'a>(
    request: &'a Parameters<QueryRequest>,
    limits: &ResourceLimits,
) -> Result<&'a str, ErrorData> {
    let QueryRequest { sql, cursor, .. } = &request.0;
    let trimmed = sql.trim().trim_end_matches(';');
    if cursor.is_some() {
        return Ok(trimmed);
    }
    validate_entry(&request.0)?;
    if trimmed.is_empty() {
        return Err(ErrorData::invalid_params("sql must not be empty", None));
    }
    if sql.len() > limits.max_query_sql_bytes {
        return Err(ErrorData::invalid_params(
            format!("sql exceeds {} bytes", limits.max_query_sql_bytes),
            error_codes::payload(ErrorCode::SqlTooLarge),
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_safety_tests.rs"]
mod safety_tests;

#[cfg(test)]
#[path = "query_pragma_tests.rs"]
mod pragma_tests;

#[cfg(test)]
#[path = "query_error_tests.rs"]
mod error_tests;

#[cfg(test)]
#[path = "query_paged_tests.rs"]
mod paged_tests;
