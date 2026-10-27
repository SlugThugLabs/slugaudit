// slugaudit-line-exception: approved-by=slugthug; reason=Everything one query statement runs under (engine value and SQL length limits, VM-step and wall-clock budgets) together with the materialisation and paging that run inside them.
//! Executing one `query` statement: the VM-step and wall-clock budgets, the
//! materialisation of its rows into the result store, and the byte-budgeted
//! page served from the stored copy.
//!
//! Split from `query.rs` so that file stays the tool contract (request and
//! response shapes, validation order, statement validation and error text)
//! and this one owns what runs under the progress handler.

use super::query_value::{Collected, PageBuilder, cell_from, read_row, values_of};

use super::context::internal_error;
use super::page::{Completeness, Page};
use super::page_size::PageSize;
use super::paginate::{PageRequest, PageSpec, Produced, RowSink, Served, paginate};
use super::query::{
    ABORT_NONE, ABORT_STEP_BUDGET, ABORT_WALL_CLOCK, describe_error, refuse_wrapped,
    single_statement_body,
};
use crate::model::{ResourceLimits, SQLITE_MAX_VALUE_BYTES};
use crate::progress::ProgressSink;
use crate::store::Take;
use crate::sync::SourceSyncManager;
use rmcp::ErrorData;
use rusqlite::Transaction;
use rusqlite::limits::Limit;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Bytes the wrapper adds around the caller's statement:
/// `SELECT * FROM (\n` and `\n) LIMIT -1 OFFSET 0` is 35. Rounded up so the engine's
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

/// The tool-level extras stored with a `query` result and returned on every
/// page: the column names, which a continuation needs to key its rows.
#[derive(Serialize, Deserialize)]
struct QueryMeta {
    columns: Vec<String>,
}

/// The producer handed to a continuation, which never produces.
fn never_produces<E>(
    _: &Transaction<'_>,
    _: &mut RowSink<'_, '_>,
) -> Result<Produced<E>, ErrorData> {
    Err(internal_error(
        "continuing a stored query result",
        "a continuation must not produce rows",
    ))
}

/// The one statement shape every query runs as. The user SQL is wrapped on
/// its own line so a trailing `-- line comment` terminates at the newline
/// rather than swallowing the closing `)`, which would otherwise produce a
/// confusing parse error. The wrapper makes anything but a `SELECT`/`WITH` a
/// syntax error. `LIMIT -1 OFFSET 0` is "everything"; paging is the store's.
fn wrapped_sql(body: &str) -> String {
    format!("SELECT * FROM (\n{body}\n) LIMIT -1 OFFSET 0")
}

/// Proves `trimmed` is one statement that runs as the wrapper's subquery and
/// returns its result column names, so the result can be created before the
/// first row exists. No row is read.
fn result_columns(
    tx: &Transaction<'_>,
    trimmed: &str,
    limits: &ResourceLimits,
) -> Result<Vec<String>, ErrorData> {
    apply_engine_limits(tx, limits)?;
    let abort_reason = Arc::new(AtomicU8::new(ABORT_NONE));
    let body = single_statement_body(tx, trimmed, &abort_reason)?;
    let statement = tx
        .prepare(&wrapped_sql(&body))
        .map_err(|error| refuse_wrapped(&error, &abort_reason))?;
    Ok(statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect())
}

/// Serves one page of `trimmed`'s result: a new request materialises every
/// row into the result store under the VM-step and wall-clock budgets and
/// serves page one; a cursor serves the stored page. Either way the page is
/// cut at `page.bytes` serialized JSON bytes, the oversized first row clipped.
///
/// A budget that runs out fails the whole call: no rows are served and no
/// result is stored.
pub(super) fn run_query(
    manager: &SourceSyncManager,
    request: PageRequest<'_>,
    trimmed: &str,
    limits: &ResourceLimits,
    page: PageSize,
) -> Result<(Collected, Page), ErrorData> {
    match request {
        PageRequest::Fresh { path, sink } => {
            fresh_page(manager, path, sink, trimmed, limits, page.bytes)
        }
        PageRequest::Cursor(token) => continuation_page(manager, token, page.bytes),
    }
}

fn fresh_page(
    manager: &SourceSyncManager,
    path: &str,
    sink: &dyn ProgressSink,
    trimmed: &str,
    limits: &ResourceLimits,
    page_bytes: usize,
) -> Result<(Collected, Page), ErrorData> {
    // The result's column count must be known before its first row, and the
    // statement is validated before any store space is claimed for it.
    let synced = super::context::ensure_synced(path, sink, manager)?;
    let columns =
        super::context::with_verified_read(&synced, |tx| result_columns(tx, trimmed, limits))?;
    let builder = RefCell::new(PageBuilder::new(columns.clone(), page_bytes));
    let spec = PageSpec {
        tool: "query",
        columns: columns.clone(),
        page_bytes,
    };
    let served = paginate(
        manager,
        &spec,
        PageRequest::Fresh { path, sink },
        |tx, rows| {
            stream_rows(tx, trimmed, limits, &builder, rows)?;
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: QueryMeta { columns },
            })
        },
        |row| builder.borrow_mut().accept(row),
    )?;
    Ok((builder.into_inner().finish(served.rows.len())?, served.page))
}

/// Serves a stored page. The store never offers the first row of a page to
/// the cut, so it is read first (together with the column names stored with
/// the result) to start the byte count from it.
fn continuation_page(
    manager: &SourceSyncManager,
    token: &str,
    page_bytes: usize,
) -> Result<(Collected, Page), ErrorData> {
    let spec = PageSpec {
        tool: "query",
        columns: Vec::new(),
        page_bytes,
    };
    let first: Served<QueryMeta> = paginate(
        manager,
        &spec,
        PageRequest::Cursor(token),
        never_produces,
        |_| Take::No,
    )?;
    let mut builder = PageBuilder::new(first.meta.columns, page_bytes);
    let [first_row] = first.rows.as_slice() else {
        return Err(internal_error(
            "continuing a stored query result",
            "the first row of a page was not returned",
        ));
    };
    builder.take_first(&values_of(first_row), first_row.ord)?;
    let builder = RefCell::new(builder);
    let served: Served<QueryMeta> = paginate(
        manager,
        &spec,
        PageRequest::Cursor(token),
        never_produces,
        |row| builder.borrow_mut().accept(row),
    )?;
    Ok((builder.into_inner().finish(served.rows.len())?, served.page))
}

/// Runs the wrapped `SELECT` inside the verified read transaction and pushes
/// every row into the store, under a fresh VM-step and wall-clock budget.
fn stream_rows(
    tx: &Transaction<'_>,
    trimmed: &str,
    limits: &ResourceLimits,
    builder: &RefCell<PageBuilder>,
    rows: &mut RowSink<'_, '_>,
) -> Result<(), ErrorData> {
    apply_engine_limits(tx, limits)?;
    let abort_reason = install_budget(tx, limits);
    let outcome = single_statement_body(tx, trimmed, &abort_reason)
        .and_then(|body| push_all(tx, &body, limits, &abort_reason, builder, rows));
    clear_budget(tx);
    outcome
}

fn push_all(
    tx: &Transaction<'_>,
    body: &str,
    limits: &ResourceLimits,
    abort_reason: &Arc<AtomicU8>,
    builder: &RefCell<PageBuilder>,
    rows: &mut RowSink<'_, '_>,
) -> Result<(), ErrorData> {
    let mut statement = tx
        .prepare(&wrapped_sql(body))
        .map_err(|error| refuse_wrapped(&error, abort_reason))?;
    let value_cap = limits.max_query_value_bytes;
    let column_count = statement.column_count();
    let mapped = statement
        .query_map([], move |row| read_row(row, column_count, value_cap))
        .map_err(|error| describe_error(&error, abort_reason))?;
    for (index, row) in mapped.enumerate() {
        let values = row.map_err(|error| describe_error(&error, abort_reason))?;
        if index == 0 {
            builder.borrow_mut().take_first(&values, 0)?;
        }
        let cells: Vec<_> = values.into_iter().map(cell_from).collect();
        rows.push_cells(&cells)?;
    }
    Ok(())
}
