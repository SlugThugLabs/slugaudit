// slugaudit-line-exception: approved-by=slugthug; reason=Everything one query statement runs under (engine value and SQL length limits, VM-step and wall-clock budgets) together with the paging that runs inside them.
//! Executing one `query` statement: the VM-step and wall-clock budgets, the
//! byte-budgeted page, and the exact total.
//!
//! Split from `query.rs` so that file stays the tool contract (request and
//! response shapes, validation order, statement validation and error text)
//! and this one owns what runs under the progress handler.

use super::query_clip::{ClippedCell, clip_row};
use super::query_value::{
    Collected, RowTotal, non_finite_cells, read_row, row_serialized_lower_bound, row_to_json,
};

use super::context::internal_error;
use super::query::{
    ABORT_NONE, ABORT_STEP_BUDGET, ABORT_WALL_CLOCK, describe_error, refuse_wrapped,
    single_statement_body,
};
use crate::model::{ResourceLimits, SQLITE_MAX_VALUE_BYTES};
use crate::util::serialized_len;
use rmcp::ErrorData;
use rusqlite::limits::Limit;
use rusqlite::types::Value as SqlValue;
use rusqlite::{MappedRows, Row, Transaction};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Bytes the page and count wrappers add around the caller's statement:
/// `SELECT * FROM (\n`, `\n) LIMIT -1 OFFSET ` and an offset of at most 19
/// digits is 54; the count wrapper is shorter. Rounded up so the engine's
/// SQL-length limit never refuses a statement the tool's own
/// `max_query_sql_bytes` check accepted.
const WRAPPER_OVERHEAD_BYTES: u64 = 64;

/// Worst-case growth of stored file text over the file bytes it came from:
/// lossy UTF-8 decoding replaces each invalid byte with U+FFFD, three bytes.
const LOSSY_UTF8_GROWTH: u64 = 3;

/// The engine limits one `query` connection runs under.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct EngineLimits {
    /// `SQLITE_LIMIT_LENGTH`: the longest string or blob SQLite will build
    /// or read, so an expression such as `hex(zeroblob(n))` fails with "too
    /// big" before allocating instead of growing to gigabytes. SQLite checks
    /// it when a stored column is read as well, so it must cover every value
    /// indexing can store: the larger of `max_query_value_bytes` and the
    /// lossy-decoded size of a `max_file_bytes` file, clamped to what the
    /// bundled SQLite can hold.
    pub(super) value_length: i32,
    /// `SQLITE_LIMIT_SQL_LENGTH`: the tool's `max_query_sql_bytes` plus the
    /// wrapper's overhead, so the engine accepts no more SQL than the tool
    /// claims to.
    pub(super) sql_length: i32,
}

/// Computes [`EngineLimits`] from the call's resource limits.
pub(super) fn engine_limits(
    limits: &ResourceLimits,
) -> Result<EngineLimits, std::num::TryFromIntError> {
    let value_bytes = u64::try_from(limits.max_query_value_bytes)?;
    let stored_bytes = limits.max_file_bytes.saturating_mul(LOSSY_UTF8_GROWTH);
    let value_length = value_bytes.max(stored_bytes).min(SQLITE_MAX_VALUE_BYTES);
    // The bundled SQLite's SQL-length ceiling is also 1,000,000,000
    // (`sqlite3_limit` reports it as the default, verified 2026-10-25).
    let sql_length = u64::try_from(limits.max_query_sql_bytes)?
        .saturating_add(WRAPPER_OVERHEAD_BYTES)
        .min(SQLITE_MAX_VALUE_BYTES);
    Ok(EngineLimits {
        value_length: i32::try_from(value_length)?,
        sql_length: i32::try_from(sql_length)?,
    })
}

/// Applies [`engine_limits`] to the connection before any caller SQL is
/// prepared.
fn apply_engine_limits(tx: &Transaction<'_>, limits: &ResourceLimits) -> Result<(), ErrorData> {
    let bounds = engine_limits(limits)
        .map_err(|error| internal_error("computing the query engine limits", error))?;
    tx.set_limit(Limit::SQLITE_LIMIT_LENGTH, bounds.value_length)
        .map_err(|error| internal_error("setting the query value-length limit", error))?;
    tx.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, bounds.sql_length)
        .map_err(|error| internal_error("setting the query SQL-length limit", error))?;
    Ok(())
}

/// `now + budget`, or `None` when that instant cannot be represented (a
/// configured budget of centuries). `None` means no wall-clock deadline: an
/// effectively unlimited budget is honoured as such instead of overflowing
/// `Instant` and panicking; the VM-step budget still bounds the work.
pub(super) fn deadline_after(budget: Duration) -> Option<Instant> {
    Instant::now().checked_add(budget)
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
    let deadline = deadline_after(limits.max_query_wall_clock);
    tx.progress_handler(
        1000,
        Some(move || {
            steps = steps.saturating_add(1000);
            if steps > max_steps {
                handler_reason.store(ABORT_STEP_BUDGET, Ordering::Relaxed);
                return true;
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
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
    apply_engine_limits(tx, limits)?;
    let abort_reason = install_budget(tx, limits);
    let page = single_statement_body(tx, trimmed, &abort_reason)
        .and_then(|body| execute_and_collect(tx, body, offset, page_bytes, limits, &abort_reason));
    clear_budget(tx);
    let (body, mut collected) = page?;
    let rows_len = i64::try_from(collected.rows.len())
        .map_err(|error| internal_error("counting the page's rows", error))?;
    let offset_rows = i64::try_from(offset)
        .map_err(|error| ErrorData::invalid_params(format!("offset {offset}: {error}"), None))?;
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
        .map_err(|error| refuse_wrapped(&error, abort_reason))?;
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
    let collected = collect_page(mapped, &column_names, offset, page_bytes, abort_reason)?;
    Ok((body, collected))
}

/// Fills one page from the mapped rows, cutting it at `page_bytes` and
/// reporting whether a row was seen past the page.
fn collect_page(
    mapped: MappedRows<'_, impl FnMut(&Row<'_>) -> rusqlite::Result<Vec<SqlValue>>>,
    column_names: &[String],
    offset: usize,
    page_bytes: usize,
    abort_reason: &Arc<AtomicU8>,
) -> Result<Collected, ErrorData> {
    let mut rows = Vec::new();
    let mut clipped = Vec::new();
    let mut non_finite = Vec::new();
    let mut total_bytes = 0usize;
    let mut size_truncated = false;
    for row in mapped {
        let values = row.map_err(|error| describe_error(&error, abort_reason))?;
        let row_index = offset.saturating_add(rows.len());
        if row_serialized_lower_bound(&values) > page_bytes {
            // The bound never exceeds the exact serialized size, so this row
            // must cross the page. Converting it would clone its TEXT and
            // hex-expand its BLOB only to drop the result, so skip that work.
            if !rows.is_empty() {
                size_truncated = true;
                break;
            }
            // The first row is always kept so paging makes progress; it needs
            // only the raw values for clipping.
            let (row, row_bytes, cells) =
                clip_oversized_row(column_names, &values, page_bytes, row_index)?;
            clipped = cells;
            total_bytes = total_bytes.saturating_add(row_bytes);
            non_finite.extend(non_finite_cells(column_names, &values, row_index));
            rows.push(row);
            continue;
        }
        let (mut value, mut row_bytes) = json_row(column_names, &values)?;
        if rows.is_empty() && row_bytes > page_bytes {
            // The first row is always kept so paging makes progress, but its
            // TEXT/BLOB cells are cut to the page and each cut is reported.
            let (row, bytes, cells) =
                clip_oversized_row(column_names, &values, page_bytes, row_index)?;
            value = row;
            row_bytes = bytes;
            clipped = cells;
        } else if !rows.is_empty() && total_bytes.saturating_add(row_bytes) > page_bytes {
            size_truncated = true;
            break;
        }
        total_bytes = total_bytes.saturating_add(row_bytes);
        non_finite.extend(non_finite_cells(column_names, &values, row_index));
        rows.push(value);
    }
    Ok(Collected {
        rows,
        truncated: size_truncated,
        rows_bytes: total_bytes,
        total: RowTotal::Exact(0),
        clipped,
        non_finite,
    })
}

/// Converts a row that alone exceeds the page into its clipped JSON, its
/// serialized size, and the cells that were cut.
fn clip_oversized_row(
    column_names: &[String],
    values: &[SqlValue],
    page_bytes: usize,
    row_index: usize,
) -> Result<(serde_json::Value, usize, Vec<ClippedCell>), ErrorData> {
    let cut = clip_row(column_names, values, page_bytes, row_index)?;
    let row_bytes = serialized_len(&cut.row)
        .map_err(|error| internal_error("measuring query row size", error))?;
    Ok((cut.row, row_bytes, cut.cells))
}

/// Converts one row to its JSON form and serialized size.
fn json_row(
    column_names: &[String],
    values: &[SqlValue],
) -> Result<(serde_json::Value, usize), ErrorData> {
    let value = row_to_json(column_names, values)?;
    let row_bytes = serialized_len(&value)
        .map_err(|error| internal_error("measuring query row size", error))?;
    Ok((value, row_bytes))
}
