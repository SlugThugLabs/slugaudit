// slugaudit-line-exception: approved-by=slugthug; reason=one tool contract owns request/response types, the execution/budget path, and the single-statement separator scanner; splitting would fragment the query tool's validation order (empty → size → statement count → freshness → budget) that the tests assert against
use super::context::{ensure_synced, guard_expected_revision, with_verified_read};
use super::query_sql_scanner::first_statement_separator;
use super::query_value::row_to_json;

use crate::error_codes::{self, ErrorCode};
use crate::model::{ResourceLimits, process_limits};
use crate::sync;
use crate::util::serialized_len;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

const MAX_ROWS: usize = 500;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_REVISION_BYTES: usize = 256;
const ABORT_NONE: u8 = 0;
const ABORT_STEP_BUDGET: u8 = 1;
const ABORT_WALL_CLOCK: u8 = 2;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Read-only SQL against the project's own database — arbitrary joins,
    /// CTEs, and window functions are fine. Only writes are rejected, and
    /// that comes from the connection itself, not from inspecting this
    /// text: there is no keyword blocklist or table allowlist.
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
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `sql` is
/// empty or too long, the query fails to parse or execute (including any
/// attempted write, which SQLite itself rejects on this connection), the
/// VM-step or wall-clock budget is exhausted, or a result value can't be
/// represented (including a single TEXT/BLOB value over the per-value cap).
pub fn query(
    request: &Parameters<QueryRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    let limits = resolve_query_limits(&request.0.path);
    query_with_limits(request, &limits, sink, manager)
}

fn resolve_query_limits(path: &str) -> ResourceLimits {
    match crate::project::find_project_root(std::path::Path::new(path)) {
        Ok(root) => crate::project::ProjectConfig::load_or_default(root.as_path())
            .apply_to_limits(*process_limits()),
        Err(_) => *process_limits(),
    }
}

/// Test-only seam: production code always goes through [`query`] with
/// [`process_limits`]; tests inject tighter limits to exercise
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
    if let Some(position) = first_statement_separator(trimmed) {
        return Err(ErrorData::invalid_params(
            format!(
                "sql contains more than one statement (separator at byte {position}); \
                 only a single read-only statement is supported"
            ),
            None,
        ));
    }

    let synced = ensure_synced(path, sink, manager)?;
    guard_expected_revision(&synced.revision_id, expected_revision.as_deref())?;
    let revision_id = synced.revision_id.clone();
    let (rows, truncated) =
        with_verified_read(&synced, |tx| run_query(tx, trimmed, *offset, limits))?;

    let next_offset = truncated.then_some(offset.saturating_add(rows.len()));

    // Row count and truncation, never the SQL text or the rows themselves.
    tracing::info!(
        revision_id,
        row_count = rows.len(),
        truncated,
        "query executed"
    );
    Ok(Json(QueryResponse {
        revision_id,
        rows,
        truncated,
        next_offset,
    }))
}

/// Executes `trimmed` under a VM-step and wall-clock budget, returning at
/// most `MAX_ROWS + 1` rows so the caller can detect row-count truncation.
fn run_query(
    tx: &Transaction<'_>,
    trimmed: &str,
    offset: usize,
    limits: &ResourceLimits,
) -> Result<(Vec<serde_json::Value>, bool), ErrorData> {
    // Progress handler aborts runaway queries after a fixed VM-step budget,
    // or after a wall-clock deadline — a query can do relatively few steps
    // that are each individually slow (disk I/O stalls), which the step
    // count alone would not catch quickly.
    let abort_reason = Arc::new(AtomicU8::new(ABORT_NONE));
    let handler_reason = Arc::clone(&abort_reason);
    let mut steps = 0_u32;
    let max_steps = limits.max_query_vm_steps;
    let deadline = Instant::now() + limits.max_query_wall_clock;
    tx.progress_handler(
        1000,
        Some(move || {
            steps = steps.saturating_add(1000);
            if steps > max_steps {
                handler_reason.store(ABORT_STEP_BUDGET, Ordering::Relaxed);
                return true;
            }
            if Instant::now() >= deadline {
                handler_reason.store(ABORT_WALL_CLOCK, Ordering::Relaxed);
                return true;
            }
            false
        }),
    );

    let result = execute_and_collect(tx, trimmed, offset, limits, &abort_reason);

    // Clear the handler so it does not outlive this call on a pooled
    // connection (we drop the connection, but be explicit).
    tx.progress_handler(0, None::<fn() -> bool>);
    result
}

/// Runs the wrapped `SELECT`, mapping each row while tagging any error with
/// the abort reason the progress handler may have already recorded.
fn execute_and_collect(
    tx: &Transaction<'_>,
    trimmed: &str,
    offset: usize,
    limits: &ResourceLimits,
    abort_reason: &Arc<AtomicU8>,
) -> Result<(Vec<serde_json::Value>, bool), ErrorData> {
    // The user SQL is wrapped on its own line so a trailing `-- line comment`
    // terminates at the newline rather than swallowing the closing `)` and
    // `LIMIT` clause, which would otherwise produce a confusing parse error.
    let wrapped = format!(
        "SELECT * FROM (\n{trimmed}\n) LIMIT {} OFFSET {offset}",
        MAX_ROWS + 1
    );
    let mut statement = tx
        .prepare(&wrapped)
        .map_err(|error| describe_error(&error, abort_reason))?;
    let column_names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let value_cap = limits.max_query_value_bytes;
    let mapped = statement
        .query_map([], move |row| row_to_json(row, &column_names, value_cap))
        .map_err(|error| describe_error(&error, abort_reason))?;
    let mut rows = Vec::new();
    let mut total_bytes = 0usize;
    let mut size_truncated = false;
    for row in mapped {
        let value = row.map_err(|error| describe_error(&error, abort_reason))?;
        let row_bytes = serialized_len(&value);
        if !rows.is_empty()
            && total_bytes.saturating_add(row_bytes) > limits.max_query_response_bytes
        {
            size_truncated = true;
            break;
        }
        total_bytes = total_bytes.saturating_add(row_bytes);
        rows.push(value);
    }
    let truncated = size_truncated || rows.len() > MAX_ROWS;
    rows.truncate(MAX_ROWS);
    Ok((rows, truncated))
}

/// Converts an aborted-query error into a message naming which budget was
/// hit, rather than surfacing SQLite's generic "interrupted" text as if it
/// were always the step budget.
fn describe_error(error: &rusqlite::Error, abort_reason: &Arc<AtomicU8>) -> ErrorData {
    let message = match abort_reason.load(Ordering::Relaxed) {
        ABORT_STEP_BUDGET => format!("query exceeded its virtual-machine step budget: {error}"),
        ABORT_WALL_CLOCK => format!("query exceeded its wall-clock time budget: {error}"),
        _ => error.to_string(),
    };
    ErrorData::invalid_params(message, None)
}

/// Returns the byte offset of the first top-level `;` in `sql`, or `None`.
/// "Top-level" means outside single-quoted strings, double-quoted
/// identifiers, backtick/bracket identifiers, and line/block comments — so
/// `SELECT ';'` and `SELECT 1 -- ;` are not mistaken for multiple
/// statements, and neither are `SELECT '--' AS x; SELECT 2` (comment
/// markers inside a literal are ordinary characters). Used only to produce
#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_safety_tests.rs"]
mod safety_tests;
