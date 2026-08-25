// slugaudit-line-exception: approved-by=agent; reason=one tool contract owns request/response types, the execution/budget path, and the single-statement separator scanner; splitting would fragment the query tool's validation order (empty → size → statement count → freshness → budget) that the tests assert against
use super::context::{ensure_synced, with_verified_read};
use super::query_value::row_to_json;
use crate::model::{ResourceLimits, process_limits};
use crate::sync;
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
    /// Combined with the response's `next_offset`, and valid only while
    /// the revision id stays the same across pages (see [`QueryResponse`]).
    #[serde(default)]
    pub offset: usize,
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
/// new revision's ordering) when it changes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    /// Revision id the page was read from. Compare across pages to detect
    /// a revision change that invalidates `next_offset` paging.
    pub revision_id: String,
    pub rows: Vec<serde_json::Value>,
    pub truncated: bool,
    /// Use this as the next request's `offset` when present. Only valid
    /// while `revision_id` is unchanged (see the paging contract above).
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
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    query_with_limits(request, process_limits(), sink, manager)
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

/// Returns the byte offset of the first top-level `;` in `sql`, or `None`.
/// "Top-level" means outside single-quoted strings, double-quoted
/// identifiers, backtick/bracket identifiers, and line/block comments — so
/// `SELECT ';'` and `SELECT 1 -- ;` are not mistaken for multiple
/// statements, and neither are `SELECT '--' AS x; SELECT 2` (comment
/// markers inside a literal are ordinary characters). Used only to produce
/// a friendlier error than SQLite's raw `near ";": syntax error` when a
/// caller sends multiple statements (which the subquery wrapper cannot
/// express). This is a UX hint, not a security boundary — the read-only
/// connection and the wrapper remain the correctness guards.
fn first_statement_separator(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_bracket = false;
    while i < bytes.len() {
        let b = bytes[i];
        // String/identifier state first: `--` and `/*` inside a literal or
        // quoted identifier are ordinary characters, not comment starts.
        // Checking comments first (the old order) let `SELECT '--x'; SELECT 2`
        // skip to end-of-line from inside the string and hide the real `;`.
        if in_single {
            if b == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 1;
                } else {
                    in_single = false;
                }
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'"' {
                if bytes.get(i + 1) == Some(&b'"') {
                    i += 1;
                } else {
                    in_double = false;
                }
            }
            i += 1;
            continue;
        }
        if in_backtick {
            if b == b'`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if in_bracket {
            if b == b']' {
                in_bracket = false;
            }
            i += 1;
            continue;
        }
        // Only outside strings/identifiers do `--` and `/*` introduce
        // comments.
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'`' => in_backtick = true,
            b'[' => in_bracket = true,
            b';' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
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
