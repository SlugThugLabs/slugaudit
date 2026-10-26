// slugaudit-line-exception: approved-by=slugthug; reason=The query tool contract in one place: request/response shapes, validation order, statement validation and every refusal message, including the PRAGMA/authorizer refusals.
use super::context::{ensure_synced, guard_expected_revision, with_verified_read};
use super::page::{Page, PageLimit};
use super::page_size;
use super::query_clip::ClippedCell;
use super::query_exec::run_query;
use super::query_value::{NonFiniteCell, RowTotal, ensure_unique_column_names};

use crate::error_codes::{self, ErrorCode};
use crate::model::ResourceLimits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_REVISION_BYTES: usize = 256;

/// Which budget, if any, the progress handler recorded before the statement
/// stopped. Shared with `query_exec`, which installs the handler.
pub(super) const ABORT_NONE: u8 = 0;
pub(super) const ABORT_STEP_BUDGET: u8 = 1;
pub(super) const ABORT_WALL_CLOCK: u8 = 2;

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
    /// itself is read-only as a second boundary. Its authorizer refuses
    /// PRAGMA, ATTACH and DETACH; of the `pragma_*` table-valued functions
    /// only schema introspection works (`pragma_table_info`,
    /// `pragma_table_xinfo`, `pragma_table_list`, `pragma_index_list`,
    /// `pragma_index_info`, `pragma_index_xinfo`, `pragma_foreign_key_list`).
    pub sql: String,
    /// Number of matching rows to skip when paging through a result.
    /// Pass the previous response's `page.next_offset`; valid only while
    /// `page.revision_id` stays the same across pages.
    #[serde(default)]
    pub offset: usize,
    /// Set to the previous page's `page.revision_id`; if the index changed
    /// since, the call is refused with `stale_revision`.
    #[serde(default)]
    pub expected_revision: Option<String>,
    /// Page size in serialized JSON bytes. Omit it for the server default
    /// (64 KiB unless configured otherwise; the response's `page.page_bytes`
    /// says what applied). A larger value returns a larger page, up to the
    /// hard ceiling `max_query_response_bytes`: a value above the ceiling is
    /// clamped, and a page the ceiling cut reports `page.limited_by` =
    /// `ceiling`. Pages are
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
/// or duplicate rows; the AI detects that boundary by comparing
/// `page.revision_id` on each page and should restart paging (or accept the
/// new revision's ordering) when it changes. Passing the previous page's
/// `revision_id` as `expected_revision` makes the server enforce that: the
/// resumed page is refused with the `stale_revision` code instead of
/// returning rows from a different ordering.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    /// Paging for `rows`, cut by bytes on a row boundary. `total` is the exact
    /// row count SQLite computes over the same statement; null only when
    /// counting ran out of budget (`total_unavailable` says why).
    pub page: Page,
    /// This page's rows, one JSON object per row with column names as keys,
    /// in the query's own row order.
    pub rows: Vec<serde_json::Value>,
    /// Why `page.total` is null (the budget that stopped the count); null
    /// whenever `page.total` is present.
    pub total_unavailable: Option<String>,
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
/// Returns an error if `request.path` isn't an active project, `sql` is
/// empty or too long, the query fails to parse or execute (including a
/// write or any other non-SELECT statement, which the wrapper refuses),
/// the VM-step or wall-clock budget is exhausted, the result's column names
/// repeat (the caller must alias them), or a result value can't be
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
    validate_query_request(request, limits)?;
    let trimmed = sql.trim().trim_end_matches(';');

    let page = page_size::resolve(*page_bytes, limits)?;

    let synced = ensure_synced(path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, expected_revision.as_deref())?;
    let revision_id = synced.revision_id.clone();
    let collected = with_verified_read(&synced, |tx| {
        run_query(tx, trimmed, *offset, page.bytes, limits)
    })?;
    let rows = collected.rows;
    let next_offset = collected.more.then_some(offset.saturating_add(rows.len()));
    let limited_by = if collected.more {
        page.byte_cut()
    } else {
        PageLimit::None
    };
    let (total, total_unavailable) = match collected.total {
        RowTotal::Exact(total) => (Some(super::page::total_of(total)?), None),
        RowTotal::Unavailable(reason) => (None, Some(reason.to_owned())),
    };

    // Row count and the page cut, never the SQL text or the rows themselves.
    tracing::info!(
        revision_id,
        row_count = rows.len(),
        complete = !collected.more,
        total,
        limited_by = ?limited_by,
        clipped_cells = collected.clipped.len(),
        non_finite_cells = collected.non_finite.len(),
        page_bytes = page.bytes,
        "query executed"
    );
    Ok(Json(QueryResponse {
        page: Page {
            returned: rows.len(),
            total,
            complete: !collected.more,
            next_offset,
            page_bytes: page.bytes,
            bytes_used: collected.bytes_used,
            limited_by,
            revision_id,
        },
        rows,
        total_unavailable,
        clipped_cells: collected.clipped,
        non_finite_cells: collected.non_finite,
    }))
}

/// Rejects oversized or empty inputs before any sync or SQL work, in the
/// order the tool contract documents.
fn validate_query_request(
    request: &Parameters<QueryRequest>,
    limits: &ResourceLimits,
) -> Result<(), ErrorData> {
    let QueryRequest {
        path,
        sql,
        offset,
        expected_revision,
        ..
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
    Ok(())
}

/// Proves `sql` is exactly one statement and returns its text without the
/// terminating `;`, so it can sit inside the wrapper's parentheses.
///
/// SQLite's own parser decides: `Connection::prepare` compiles the first
/// statement and, when input remains after it, prepares that remainder and
/// fails with `Error::MultipleStatement` only if the remainder holds a real
/// statement (rusqlite 0.37.0 `src/lib.rs:770-775`). Whitespace, comments and
/// bare `;` after the first statement are therefore accepted, and a `;`
/// inside a literal, quoted identifier or comment never counts. No SQL is
/// lexed here. `expanded_sql` (no parameters are bound) is the statement text
/// SQLite consumed, which excludes any trailing comment after the `;`.
pub(super) fn single_statement_body(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    abort_reason: &Arc<AtomicU8>,
) -> Result<String, ErrorData> {
    let statement = tx.prepare(sql).map_err(|error| match error {
        rusqlite::Error::MultipleStatement => ErrorData::invalid_params(
            "sql contains more than one statement; \
             only a single read-only statement is supported",
            None,
        ),
        other => describe_error(&other, abort_reason),
    })?;
    // Check the caller's own projection, before the wrapper turns it into a
    // subquery: SQLite renames the second `id` of a subquery to `id:1`, so
    // checking only the wrapped result would miss the duplicate and hand back
    // a renamed key the caller never asked for.
    let column_names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    ensure_unique_column_names(&column_names)?;
    let text = statement.expanded_sql().ok_or_else(|| {
        ErrorData::invalid_params("sql must contain a statement, not only comments", None)
    })?;
    Ok(text.trim_end().trim_end_matches(';').to_owned())
}

/// The refusal for a statement that SQLite parses on its own but cannot run
/// as the wrapper's subquery: it is valid SQL of another kind. Only the
/// wrapper's syntax error is classified this way, because the bare statement
/// already compiled, so a syntax error can come only from the wrapping; any
/// other failure keeps [`describe_error`]'s text.
pub(super) fn refuse_wrapped(error: &rusqlite::Error, abort_reason: &Arc<AtomicU8>) -> ErrorData {
    if !matches!(error, rusqlite::Error::SqlInputError { .. }) {
        return describe_error(error, abort_reason);
    }
    ErrorData::invalid_params(
        format!(
            "sql was refused: `query` accepts a single read-only SELECT/WITH statement, and \
             this is another kind of statement (PRAGMA, EXPLAIN, a write, DDL or a \
             transaction command), so it cannot run as a subquery (SQLite: {error})"
        ),
        None,
    )
}

/// Converts an aborted-query error into a message naming which budget was
/// hit, rather than surfacing SQLite's generic "interrupted" text as if it
/// were always the step budget. An authorizer denial (PRAGMA, ATTACH,
/// DETACH, at prepare time or from a `pragma_*` table function during the
/// step) becomes the refusal that says what `query` accepts; SQLite's own
/// error is kept in the message and logged here, once.
pub(super) fn describe_error(error: &rusqlite::Error, abort_reason: &Arc<AtomicU8>) -> ErrorData {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::AuthorizationForStatementDenied) {
        // SQLite's text for this code is the static "not authorized"; the
        // SQL itself is never logged.
        tracing::info!(error = %error, "query refused by the read-only connection's authorizer");
        return ErrorData::invalid_params(
            format!(
                "sql was refused: `query` accepts a single read-only SELECT/WITH statement. \
                 The read-only connection does not authorize PRAGMA statements, ATTACH or \
                 DETACH, and of the pragma table-valued functions only schema introspection \
                 is allowed (pragma_table_info, pragma_table_xinfo, pragma_table_list, \
                 pragma_index_list, pragma_index_info, pragma_index_xinfo, \
                 pragma_foreign_key_list) (SQLite: {error})"
            ),
            None,
        );
    }
    match abort_reason.load(Ordering::Relaxed) {
        ABORT_STEP_BUDGET => ErrorData::invalid_params(
            format!("query exceeded its virtual-machine step budget: {error}"),
            None,
        ),
        ABORT_WALL_CLOCK => ErrorData::invalid_params(
            format!("query exceeded its wall-clock time budget: {error}"),
            None,
        ),
        _ => ErrorData::invalid_params(error.to_string(), sqlite_error_payload(error)),
    }
}

/// `sql_error` when SQLite itself failed to compile or run the statement;
/// `None` for a failure raised on this side of the boundary (an oversized
/// value, a type conversion), whose message already names its own fix.
fn sqlite_error_payload(error: &rusqlite::Error) -> Option<serde_json::Value> {
    match error {
        rusqlite::Error::SqliteFailure(..) | rusqlite::Error::SqlInputError { .. } => {
            error_codes::payload(ErrorCode::SqlError)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_safety_tests.rs"]
mod safety_tests;

#[cfg(test)]
#[path = "query_error_tests.rs"]
mod error_tests;
