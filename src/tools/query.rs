// slugaudit-line-exception: approved-by=slugthuglabs; reason=The query tool contract in one place: request/response shapes, validation order, statement validation and every refusal message, including the PRAGMA refusal.
use super::page::{LegacyPaging, Page, PageLimit};
use super::page_size::{self, PageSize};
use super::paginate::page_request;
use super::query_clip::ClippedCell;
use super::query_exec::run_query;
use super::query_value::{NonFiniteCell, ensure_unique_column_names};

use crate::error_codes::{self, ErrorCode};
use crate::model::ResourceLimits;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use super::MAX_PATH_BYTES;

/// Which budget, if any, the progress handler recorded before the statement
/// stopped. Shared with `query_exec`, which installs the handler.
pub(super) const ABORT_NONE: u8 = 0;
pub(super) const ABORT_STEP_BUDGET: u8 = 1;
pub(super) const ABORT_WALL_CLOCK: u8 = 2;

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
    // A continuation names no project and reads no limit: its page budget and
    // the ceiling an explicit `page_bytes` is clamped to are those stored with
    // the result, so the process limits only fill the argument. A new request
    // runs under its project's. Without either, `page_request` refuses the
    // call below.
    let limits = match (&request.0.cursor, &request.0.path) {
        (None, Some(path)) => super::resolve_limits(path)?,
        _ => super::process_limits_for_tool()?,
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

    let page = page_size::resolve(*page_bytes, limits)?;
    let paging = page_request(cursor.as_deref(), path.as_deref(), sink)?;
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

/// Rejects oversized or empty inputs before any sync or SQL work, in the
/// order the tool contract documents, and returns the SQL text without
/// surrounding whitespace or terminating `;`. A continuation carries nothing
/// else to check: its `path` and `sql` are ignored.
fn validate_query_request<'a>(
    request: &'a Parameters<QueryRequest>,
    limits: &ResourceLimits,
) -> Result<&'a str, ErrorData> {
    let QueryRequest {
        path, sql, cursor, ..
    } = &request.0;
    let trimmed = sql.trim().trim_end_matches(';');
    if cursor.is_some() {
        return Ok(trimmed);
    }
    if path
        .as_deref()
        .is_some_and(|path| crate::util::exceeds_input_bytes(path, MAX_PATH_BYTES))
    {
        return Err(ErrorData::invalid_params(
            format!("path exceeds {MAX_PATH_BYTES} bytes"),
            None,
        ));
    }
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

/// Proves `sql` is exactly one statement and returns its text without the
/// terminating `;`, so it can sit inside the wrapper's parentheses.
///
/// SQLite's own parser decides: `Connection::prepare` compiles the first
/// statement and, when input remains after it, prepares that remainder and
/// fails with `Error::MultipleStatement` only if the remainder holds a real
/// statement (rusqlite 0.37.0 `src/lib.rs:770-775`). Whitespace, comments and
/// bare `;` after the first statement are therefore accepted, and a `;`
/// inside a literal, quoted identifier or comment never counts. No SQL is
/// lexed for the statement body. `expanded_sql` (no parameters are bound) is the statement text
/// SQLite consumed, which excludes any trailing comment after the `;`.
pub(super) fn single_statement_body(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    abort_reason: &Arc<AtomicU8>,
) -> Result<String, ErrorData> {
    // Several PRAGMAs take effect while being *prepared* (the heap limits,
    // `temp_store_directory`), and `prepare` also prepares every statement
    // after the first to detect `MultipleStatement`. So every statement's
    // first keyword is checked before the text reaches the parser. Every
    // other non-SELECT kind has no prepare-time effect and is refused by the
    // wrapper below.
    if any_statement_is_pragma(sql) {
        return Err(ErrorData::invalid_params(
            "sql was refused: `query` accepts a single read-only SELECT/WITH statement, \
             not PRAGMA (a PRAGMA can change process state while being prepared)",
            None,
        ));
    }
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
    if !statement.readonly() {
        return Err(ErrorData::invalid_params(
            "sql was refused: `query` accepts a single read-only SELECT/WITH statement",
            None,
        ));
    }
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
/// were always the step budget. SQLite's own error is kept in the message
/// and logged here, once.
pub(super) fn describe_error(error: &rusqlite::Error, abort_reason: &Arc<AtomicU8>) -> ErrorData {
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

/// True when any statement of `sql` (split at each `;` outside a literal,
/// quoted identifier or comment) starts with `PRAGMA`.
fn any_statement_is_pragma(sql: &str) -> bool {
    let mut rest = sql;
    loop {
        if first_keyword_is_pragma(rest) {
            return true;
        }
        match statement_end(rest).and_then(|end| rest.get(end + 1..)) {
            Some(after) => rest = after,
            None => return false,
        }
    }
}

/// Byte index of the first `;` in `sql` that ends a statement: one outside a
/// single-, double- or backtick-quoted string, a `[bracket]` identifier, a
/// `--` line comment and a `/* */` block comment. `None` when no statement
/// ends in `sql`. Every delimiter is ASCII, so the index is a char boundary.
fn statement_end(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let (opener, closing): (usize, &[u8]) = match byte {
            b';' => return Some(index),
            b'\'' => (1, b"'"),
            b'"' => (1, b"\""),
            b'`' => (1, b"`"),
            b'[' => (1, b"]"),
            b'-' if bytes.get(index + 1) == Some(&b'-') => (2, b"\n"),
            b'/' if bytes.get(index + 1) == Some(&b'*') => (2, b"*/"),
            _ => {
                index += 1;
                continue;
            }
        };
        let body = index + opener;
        let close = bytes
            .get(body..)?
            .windows(closing.len())
            .position(|window| window == closing)?;
        index = body + close + closing.len();
    }
    None
}

/// True when `sql`'s first token, after leading whitespace and comments, is
/// the `PRAGMA` keyword (case-insensitive).
fn first_keyword_is_pragma(sql: &str) -> bool {
    let mut rest = sql;
    loop {
        rest = rest.trim_start_matches(|character: char| character.is_ascii_whitespace());
        if let Some(comment) = rest.strip_prefix("--") {
            // A line comment ends at the newline; without one it runs to the
            // end of the statement, leaving no keyword.
            rest = comment.split_once('\n').map_or("", |(_, after)| after);
            continue;
        }
        if let Some(comment) = rest.strip_prefix("/*") {
            match comment.split_once("*/") {
                Some((_, after)) => {
                    rest = after;
                    continue;
                }
                None => return false,
            }
        }
        break;
    }
    rest.split(|character: char| !character.is_ascii_alphabetic())
        .next()
        .is_some_and(|keyword| keyword.eq_ignore_ascii_case("pragma"))
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
#[path = "query_pragma_tests.rs"]
mod pragma_tests;

#[cfg(test)]
#[path = "query_error_tests.rs"]
mod error_tests;

#[cfg(test)]
#[path = "query_paged_tests.rs"]
mod paged_tests;
