//! Executing one `query` statement: the VM-step and wall-clock budgets, the
//! materialisation of its rows into the result store, and the byte-budgeted
//! page served from the stored copy.

use super::page_builder::{Collected, PageBuilder};
use super::sql::{
    ABORT_NONE, ABORT_STEP_BUDGET, ABORT_WALL_CLOCK, describe_error, refuse_wrapped,
    single_statement_body,
};
use super::value::{cell_from, read_row};
use crate::model::{ResourceLimits, SQLITE_MAX_VALUE_BYTES};
use crate::store::StoredRow;
use crate::sync::SourceSyncManager;
use crate::tools::context::internal_error;
use crate::tools::page::{Completeness, Page, PageSize};
use crate::tools::paginate::{PageRequest, PageSpec, Produced, RowSink, paginate_rows};
use crate::tools::tool_limits::deadline_after;
use rmcp::ErrorData;
use rusqlite::Transaction;
use rusqlite::limits::Limit;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

const WRAPPER_OVERHEAD_BYTES: u64 = 64;
const STORED_VALUE_GROWTH: u64 = 1;
const PROGRESS_INTERVAL: i32 = 1000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct EngineLimits {
    pub(super) value_length: i32,
    pub(super) sql_length: i32,
}

pub(super) fn engine_limits(
    limits: &ResourceLimits,
) -> Result<EngineLimits, std::num::TryFromIntError> {
    let value_bytes = u64::try_from(limits.max_query_value_bytes)?;
    // KEEP saturating: the product is clamped to the engine ceiling just below
    let stored_bytes = limits.max_file_bytes.saturating_mul(STORED_VALUE_GROWTH);
    let value_length = value_bytes.max(stored_bytes).min(SQLITE_MAX_VALUE_BYTES);
    // KEEP saturating: the sum is clamped to the engine ceiling just below
    let sql_length = u64::try_from(limits.max_query_sql_bytes)?
        .saturating_add(WRAPPER_OVERHEAD_BYTES)
        .min(SQLITE_MAX_VALUE_BYTES);
    Ok(EngineLimits {
        value_length: i32::try_from(value_length)?,
        sql_length: i32::try_from(sql_length)?,
    })
}

fn apply_engine_limits(tx: &Transaction<'_>, limits: &ResourceLimits) -> Result<(), ErrorData> {
    let bounds = engine_limits(limits)
        .map_err(|error| internal_error("computing the query engine limits", error))?;
    tx.set_limit(Limit::SQLITE_LIMIT_LENGTH, bounds.value_length)
        .map_err(|error| internal_error("setting the query value-length limit", error))?;
    tx.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, bounds.sql_length)
        .map_err(|error| internal_error("setting the query SQL-length limit", error))?;
    Ok(())
}

fn install_budget(tx: &Transaction<'_>, limits: &ResourceLimits) -> Arc<AtomicU8> {
    let abort_reason = Arc::new(AtomicU8::new(ABORT_NONE));
    let handler_reason = Arc::clone(&abort_reason);
    let mut steps = 0_u32;
    let max_steps = limits.max_query_vm_steps;
    let deadline = deadline_after(limits.max_query_wall_clock);
    tx.progress_handler(
        PROGRESS_INTERVAL,
        Some(move || {
            let Some(counted) = steps
                .checked_add(PROGRESS_INTERVAL.unsigned_abs())
                .filter(|counted| *counted <= max_steps)
            else {
                handler_reason.store(ABORT_STEP_BUDGET, Ordering::Relaxed);
                return true;
            };
            steps = counted;
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                handler_reason.store(ABORT_WALL_CLOCK, Ordering::Relaxed);
                return true;
            }
            false
        }),
    );
    abort_reason
}

fn clear_budget(tx: &Transaction<'_>) {
    tx.progress_handler(0, None::<fn() -> bool>);
}

fn wrapped_sql(body: &str) -> String {
    format!("SELECT * FROM (\n{body}\n) LIMIT -1 OFFSET 0")
}

pub(super) fn run_query(
    manager: &SourceSyncManager,
    request: PageRequest<'_>,
    trimmed: &str,
    limits: &ResourceLimits,
    size: PageSize,
    requested_bytes: Option<usize>,
) -> Result<(Collected, Page), ErrorData> {
    let mut builder = PageBuilder::new();
    let spec = PageSpec {
        tool: "query",
        columns: Vec::new(),
        page_bytes: size.bytes,
        requested_page_bytes: requested_bytes,
        ceiling: limits.max_query_response_bytes,
        max_rows: None,
    };
    let served = paginate_rows(
        manager,
        &spec,
        request,
        |tx, rows| {
            stream_rows(tx, trimmed, limits, rows)?;
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: (),
            })
        },
        |ctx| {
            builder.configure(ctx.columns.to_vec(), ctx.page_bytes)?;
            let builder = &mut builder;
            Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                builder.cut_row(row, taken, bytes_so_far)
            })
        },
    )?;
    Ok((builder.into_collected(served.rows), served.page))
}

fn stream_rows(
    tx: &Transaction<'_>,
    trimmed: &str,
    limits: &ResourceLimits,
    rows: &mut RowSink<'_, '_>,
) -> Result<(), ErrorData> {
    apply_engine_limits(tx, limits)?;
    let abort_reason = install_budget(tx, limits);
    let outcome = single_statement_body(tx, trimmed, &abort_reason)
        .and_then(|body| push_all(tx, &body, limits, &abort_reason, rows));
    clear_budget(tx);
    outcome
}

fn push_all(
    tx: &Transaction<'_>,
    body: &str,
    limits: &ResourceLimits,
    abort_reason: &Arc<AtomicU8>,
    rows: &mut RowSink<'_, '_>,
) -> Result<(), ErrorData> {
    let mut statement = tx
        .prepare(&wrapped_sql(body))
        .map_err(|error| refuse_wrapped(&error, abort_reason))?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    rows.set_columns(names)?;
    let value_cap = limits.max_query_value_bytes;
    let column_count = statement.column_count();
    let mapped = statement
        .query_map([], move |row| read_row(row, column_count, value_cap))
        .map_err(|error| describe_error(&error, abort_reason))?;
    for row in mapped {
        let values = row.map_err(|error| describe_error(&error, abort_reason))?;
        let cells: Vec<_> = values.into_iter().map(cell_from).collect();
        rows.push_cells(&cells)?;
    }
    Ok(())
}
