//! Executing one `query` statement: the single-statement proof, the VM-step
//! and wall-clock budgets, the byte-budgeted page, and the exact total.
//!
//! Split from `query.rs` so that file stays the tool contract (request and
//! response shapes, validation order) and this one owns what runs under the
//! progress handler.

use super::query_clip::{ClippedCell, clip_row};
use super::query_value::{read_row, row_to_json};

use super::context::internal_error;
use crate::model::ResourceLimits;
use crate::util::serialized_len;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

const ABORT_NONE: u8 = 0;
const ABORT_STEP_BUDGET: u8 = 1;
const ABORT_WALL_CLOCK: u8 = 2;

/// The size of the whole result, independent of the page that was returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowTotal {
    /// Exact row count of the whole result.
    Exact(i64),
    /// Counting did not finish; the static reason names the budget that
    /// stopped it. No estimate is ever substituted.
    Unavailable(&'static str),
}

/// One page of a result plus the facts that make it quotable.
pub(super) struct Collected {
    pub(super) rows: Vec<serde_json::Value>,
    /// True when more rows remain past this page.
    pub(super) truncated: bool,
    /// Serialized JSON bytes of `rows`, measured per row.
    pub(super) rows_bytes: usize,
    pub(super) total: RowTotal,
    /// The cells cut from the first row because it alone exceeded the page.
    pub(super) clipped: Vec<ClippedCell>,
}

/// Installs the VM-step and wall-clock progress handler and returns the cell
/// that records which budget fired. Each call starts a fresh budget.
fn install_budget(tx: &Transaction<'_>, limits: &ResourceLimits) -> Arc<AtomicU8> {
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
    abort_reason
}

/// Clears the handler so it does not outlive its call on a pooled
/// connection (we drop the connection, but be explicit).
fn clear_budget(tx: &Transaction<'_>) {
    tx.progress_handler(0, None::<fn() -> bool>);
}

/// Executes `trimmed` under a VM-step and wall-clock budget, returning the
/// rows that fit `page_bytes` and the exact size of the whole result (or the
/// reason it could not be counted).
///
/// The total costs nothing extra when the page already ends the result: then
/// it is `offset + rows` exactly. Only a cut page (or an `offset` past the
/// end) is counted, by SQLite, under its own fresh budget — a count that
/// exhausts it reports why instead of failing the call.
pub(super) fn run_query(
    tx: &Transaction<'_>,
    trimmed: &str,
    offset: usize,
    page_bytes: usize,
    limits: &ResourceLimits,
) -> Result<Collected, ErrorData> {
    let abort_reason = install_budget(tx, limits);
    let page = single_statement_body(tx, trimmed, &abort_reason)
        .and_then(|body| execute_and_collect(tx, body, offset, page_bytes, limits, &abort_reason));
    clear_budget(tx);
    let (body, mut collected) = page?;
    let rows_len = i64::try_from(collected.rows.len()).unwrap_or(i64::MAX);
    let offset_rows = i64::try_from(offset).unwrap_or(i64::MAX);
    collected.total = if !collected.truncated && (offset == 0 || rows_len > 0) {
        RowTotal::Exact(offset_rows.saturating_add(rows_len))
    } else {
        let count_budget = install_budget(tx, limits);
        let counted = count_rows(tx, &body, &count_budget);
        clear_budget(tx);
        counted?
    };
    Ok(collected)
}

/// `SELECT count(*)` over the same single-statement body, wrapped like the
/// page query. A spent budget becomes [`RowTotal::Unavailable`]; any other
/// failure fails the call.
fn count_rows(
    tx: &Transaction<'_>,
    body: &str,
    abort_reason: &Arc<AtomicU8>,
) -> Result<RowTotal, ErrorData> {
    let counted = tx.query_row(&format!("SELECT count(*) FROM (\n{body}\n)"), [], |row| {
        row.get::<_, i64>(0)
    });
    match counted {
        Ok(total) => Ok(RowTotal::Exact(total)),
        Err(error) => match abort_reason.load(Ordering::Relaxed) {
            ABORT_STEP_BUDGET => {
                tracing::warn!(error = %error, "query total_rows count exceeded its step budget");
                Ok(RowTotal::Unavailable(
                    "counting the whole result exceeded the VM-step budget (max_query_vm_steps)",
                ))
            }
            ABORT_WALL_CLOCK => {
                tracing::warn!(error = %error, "query total_rows count exceeded its time budget");
                Ok(RowTotal::Unavailable(
                    "counting the whole result exceeded the wall-clock budget (max_query_wall_clock)",
                ))
            }
            _ => Err(describe_error(&error, abort_reason)),
        },
    }
}

/// Runs the wrapped `SELECT`, mapping each row while tagging any error with
/// the abort reason the progress handler may have already recorded. Returns
/// the single-statement body too, for the count.
fn execute_and_collect(
    tx: &Transaction<'_>,
    body: String,
    offset: usize,
    page_bytes: usize,
    limits: &ResourceLimits,
    abort_reason: &Arc<AtomicU8>,
) -> Result<(String, Collected), ErrorData> {
    // The user SQL is wrapped on its own line so a trailing `-- line comment`
    // terminates at the newline rather than swallowing the closing `)` and
    // `LIMIT` clause, which would otherwise produce a confusing parse error.
    // `LIMIT -1` is "no limit": SQLite requires a LIMIT before OFFSET, and the
    // page is cut only by the byte budget below.
    let wrapped = format!("SELECT * FROM (\n{body}\n) LIMIT -1 OFFSET {offset}");
    let mut statement = tx
        .prepare(&wrapped)
        .map_err(|error| describe_error(&error, abort_reason))?;
    let column_names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let value_cap = limits.max_query_value_bytes;
    let column_count = column_names.len();
    let mapped = statement
        .query_map([], move |row| read_row(row, column_count, value_cap))
        .map_err(|error| describe_error(&error, abort_reason))?;
    let mut rows = Vec::new();
    let mut clipped = Vec::new();
    let mut total_bytes = 0usize;
    let mut size_truncated = false;
    for row in mapped {
        let values = row.map_err(|error| describe_error(&error, abort_reason))?;
        let mut value = row_to_json(&column_names, &values);
        let mut row_bytes = serialized_len(&value)
            .map_err(|error| internal_error("measuring query row size", error))?;
        if rows.is_empty() && row_bytes > page_bytes {
            // The first row is always kept so paging makes progress, but its
            // TEXT/BLOB cells are cut to the page and each cut is reported.
            let row_index = offset.saturating_add(rows.len());
            let cut = clip_row(&column_names, &values, page_bytes, row_index)
                .map_err(|error| internal_error("clipping an oversized query row", error))?;
            value = cut.row;
            clipped = cut.cells;
            row_bytes = serialized_len(&value)
                .map_err(|error| internal_error("measuring query row size", error))?;
        } else if !rows.is_empty() && total_bytes.saturating_add(row_bytes) > page_bytes {
            size_truncated = true;
            break;
        }
        total_bytes = total_bytes.saturating_add(row_bytes);
        rows.push(value);
    }
    Ok((
        body,
        Collected {
            rows,
            truncated: size_truncated,
            rows_bytes: total_bytes,
            total: RowTotal::Exact(0),
            clipped,
        },
    ))
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
fn single_statement_body(
    tx: &Transaction<'_>,
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
    let text = statement.expanded_sql().ok_or_else(|| {
        ErrorData::invalid_params("sql must contain a statement, not only comments", None)
    })?;
    Ok(text.trim_end().trim_end_matches(';').to_owned())
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
