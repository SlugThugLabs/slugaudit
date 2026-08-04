use super::context::{SyncRecencyCache, ensure_synced, with_verified_read};
use super::query_value::row_to_json;
use crate::model::ResourceLimits;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

const MAX_ROWS: usize = 500;
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
    #[serde(default)]
    pub offset: usize,
}

/// Arbitrary read-only query results: one JSON object per row, column
/// names as keys. This is the general-purpose tool — search, symbol/
/// import/diagnostic lookup, dependency traversal, and source retrieval
/// all reach through it as ordinary `SELECT`s against the schema.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    pub revision_id: String,
    pub rows: Vec<serde_json::Value>,
    pub truncated: bool,
    /// Use this as the next request's `offset` when present.
    pub next_offset: Option<usize>,
}

/// Borrowed mirror of `QueryResponse`, used to measure the exact serialized
/// size of a candidate response — struct and array framing included —
/// without cloning the row vector.
#[derive(Serialize)]
struct QueryResponseView<'a> {
    revision_id: &'a str,
    rows: &'a [serde_json::Value],
    truncated: bool,
    next_offset: Option<usize>,
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
    cache: &SyncRecencyCache,
) -> Result<Json<QueryResponse>, ErrorData> {
    query_with_limits(request, &ResourceLimits::default(), cache)
}

/// Test-only seam: production code always goes through [`query`] with
/// [`ResourceLimits::default`]; tests inject tighter limits to exercise
/// truncation and budget paths without waiting out production-sized caps.
/// Private (not `pub`), but visible to the `tests` submodule below like any
/// other item in this module.
fn query_with_limits(
    request: &Parameters<QueryRequest>,
    limits: &ResourceLimits,
    cache: &SyncRecencyCache,
) -> Result<Json<QueryResponse>, ErrorData> {
    let QueryRequest { path, sql, offset } = &request.0;
    let trimmed = sql.trim().trim_end_matches(';');
    if trimmed.is_empty() {
        return Err(ErrorData::invalid_params("sql must not be empty", None));
    }
    if sql.len() > limits.max_query_sql_bytes {
        return Err(ErrorData::invalid_params(
            format!("sql exceeds {} bytes", limits.max_query_sql_bytes),
            None,
        ));
    }

    let synced = ensure_synced(path, cache)?;
    let revision_id = synced.revision_id.clone();
    let (mut rows, mut truncated) =
        with_verified_read(&synced, |tx| run_query(tx, trimmed, *offset, limits))?;

    shrink_to_fit(
        &revision_id,
        &mut rows,
        &mut truncated,
        *offset,
        limits.max_query_response_bytes,
    )?;
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
    for row in mapped {
        rows.push(row.map_err(|error| describe_error(&error, abort_reason))?);
    }
    let truncated = rows.len() > MAX_ROWS;
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

/// Enforces the full serialized `QueryResponse` size, framing included, by
/// dropping rows from the end and re-measuring until the candidate fits.
/// Correctness over raw performance: `MAX_ROWS` caps the work at 500 rows.
fn shrink_to_fit(
    revision_id: &str,
    rows: &mut Vec<serde_json::Value>,
    truncated: &mut bool,
    offset: usize,
    max_bytes: usize,
) -> Result<(), ErrorData> {
    loop {
        let view = QueryResponseView {
            revision_id,
            rows,
            truncated: *truncated,
            next_offset: (*truncated).then_some(offset.saturating_add(rows.len())),
        };
        let encoded_len = serde_json::to_vec(&view)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
            .len();
        if encoded_len <= max_bytes || rows.is_empty() {
            return Ok(());
        }
        rows.pop();
        *truncated = true;
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
