pub(in crate::tools) mod clip;
pub(in crate::tools) mod exec;
mod json_text;
mod page_builder;
mod sql;
pub(in crate::tools) mod value;

pub use clip::ClippedCell;
pub use value::NonFiniteCell;

#[cfg(test)]
use super::MAX_PATH_BYTES;
use super::check_optional_path_bytes;
use super::page::{LegacyPaging, Page, PageSize};
use super::paginate::page_request;
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::ResourceLimits;
use crate::sync;
use exec::run_query;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// hard ceiling `max_query_response_bytes`. Pages are cut on a row
    /// boundary; when the first row alone is larger than the page, its
    /// TEXT/BLOB cells are clipped to fit and listed in `clipped_cells` with
    /// the `substr` expression that continues each one. May change between
    /// pages.
    #[serde(default)]
    pub page_bytes: Option<usize>,
    /// The `offset` and `expected_revision` parameters, accepted only to refuse
    /// them naming `cursor`.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
}

/// Arbitrary read-only query results: one JSON object per row, column
/// names as keys.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    /// Paging for `rows`, cut by bytes on a row boundary. `total` is the exact
    /// row count of the whole stored result.
    pub page: Page,
    /// This page's rows, one JSON object per row with column names as keys,
    /// in the query's own row order.
    pub rows: Vec<serde_json::Value>,
    /// The TEXT/BLOB cells cut from the first row because the row alone was
    /// larger than `page_bytes`; empty when nothing was cut.
    pub clipped_cells: Vec<ClippedCell>,
    /// REAL cells that are `null` in `rows` only because JSON cannot carry an
    /// infinity or NaN. Empty when every REAL is finite.
    pub non_finite_cells: Vec<NonFiniteCell>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project (or neither it
/// nor a `cursor` is given), `sql` is empty or too long, the query fails to
/// parse or execute, the VM-step or wall-clock budget is exhausted, the
/// result's column names repeat, a result value exceeds `max_query_value_bytes`,
/// the request carries `offset`/`expected_revision`, or the cursor is unknown
/// or expired.
pub fn query(
    request: &Parameters<QueryRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<QueryResponse>, ErrorData> {
    request.0.legacy_paging.refuse()?;
    validate_entry(&request.0)?;
    let limits = match (&request.0.cursor, &request.0.path) {
        (None, Some(path)) => super::resolve_limits(path)?,
        _ => ResourceLimits::default(),
    };
    query_with_limits(request, &limits, sink, manager)
}

pub(super) fn query_with_limits(
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
    let page = PageSize::resolve(*page_bytes, limits)?;
    let continuation = cursor.is_some();

    let (collected, stored_page) = run_query(manager, paging, trimmed, limits, page, *page_bytes)?;
    let complete = stored_page.complete;

    tracing::info!(
        revision_id = stored_page.revision_id,
        row_count = collected.rows.len(),
        complete,
        total = stored_page.total,
        continuation,
        limited_by = ?stored_page.limited_by,
        clipped_cells = collected.clipped.len(),
        non_finite_cells = collected.non_finite.len(),
        page_bytes = stored_page.page_bytes,
        "query executed"
    );
    Ok(Json(QueryResponse {
        page: stored_page,
        rows: collected.rows,
        clipped_cells: collected.clipped,
        non_finite_cells: collected.non_finite,
    }))
}

fn validate_entry(request: &QueryRequest) -> Result<(), ErrorData> {
    if request.cursor.is_none() {
        check_optional_path_bytes(request.path.as_deref())?;
    }
    Ok(())
}

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
        return Err(invalid_params_with_code(
            "sql must not be empty",
            ErrorCode::InvalidInput,
        ));
    }
    if sql.len() > limits.max_query_sql_bytes {
        return Err(invalid_params_with_code(
            format!("sql exceeds {} bytes", limits.max_query_sql_bytes),
            ErrorCode::SqlTooLarge,
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod budget_tests;

#[cfg(test)]
mod clip_lines_tests;

#[cfg(test)]
mod clip_tests;

#[cfg(test)]
mod error_tests;

#[cfg(test)]
mod page_tests;

#[cfg(test)]
mod paged_budget_tests;

#[cfg(test)]
mod paged_tests;

#[cfg(test)]
mod safety_limits_tests;

#[cfg(test)]
mod safety_tests;

#[cfg(test)]
mod tests;
