//! The compact `leads` enumeration: `mechanic`, `path`, `line` triples with
//! no per-row text, over one mechanic or the whole catalogue at once.
//!
//! Full-mode rows are read one mechanic at a time (`leads_query.rs`) because
//! each row carries its `detail` text and a page is therefore almost always
//! about a single mechanic. Compact rows drop that text, so the whole
//! mechanical surface is affordable in one response — and, unlike a page per
//! mechanic, a single call has to page across mechanics while keeping the
//! cumulative file count honest.
//!
//! That is why every read here walks one `UNION ALL` over the selected
//! mechanics rather than concatenating per-mechanic reads in Rust: the row
//! order (`ord`, then `path`, then `line`) is defined by the statement, so a
//! page boundary, an offset, and `files_covered` all agree without a second
//! ordering to keep in sync. `ord` is the position in `MECHANICS`; the
//! mechanic id itself is spliced as a literal from the static catalogue
//! (never from caller input), which is what keeps each branch self-identifying
//! without a lookup table in the response.
//!
//! Every caller value — the `pattern` glob, the limit, the offset — is bound
//! as a parameter, exactly as in `leads_query.rs`.

use super::leads::CompactLeadRow;
use super::leads_mechanics::{MECHANICS, Mechanic};
use super::leads_query::{Page, compile_pattern, fill_page, int_bind};
use rmcp::ErrorData;
use rusqlite::types::Value;
use rusqlite::{Transaction, params_from_iter};

/// The mechanics a compact call enumerates: the requested one, or the whole
/// catalogue in declaration order.
pub(super) fn scope(selected: Option<&Mechanic>) -> Vec<&Mechanic> {
    match selected {
        Some(mechanic) => vec![mechanic],
        None => MECHANICS.iter().collect(),
    }
}

/// One `SELECT` per mechanic, each tagging its rows with the mechanic id and
/// its catalogue position. The id is a compile-time constant from
/// [`MECHANICS`], never a caller value.
fn branches(scope: &[&Mechanic], matched: bool) -> String {
    scope
        .iter()
        .enumerate()
        .map(|(ordinal, mechanic)| {
            format!(
                "SELECT {ordinal} AS ord, '{}' AS mechanic, {} AS path, {} AS line FROM {}{}",
                mechanic.id,
                mechanic.path_expr(),
                mechanic.line_expr(),
                mechanic.read_from(),
                mechanic.where_clause(matched)
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ")
}

/// The paged triple query: catalogue order, then the mechanic's own file/line
/// order. Placeholders bind in textual order — one pattern glob per branch,
/// then the limit, then the offset.
fn rows_sql(scope: &[&Mechanic], matched: bool) -> String {
    format!(
        "SELECT mechanic, path, line FROM ({}) ORDER BY ord, path, line LIMIT ? OFFSET ?",
        branches(scope, matched)
    )
}

/// Distinct files represented in the first `LIMIT` rows of the same ordered
/// result `rows_sql` walks — the cumulative file count after a page across
/// every mechanic in scope.
fn covered_sql(scope: &[&Mechanic], matched: bool) -> String {
    format!(
        "SELECT count(DISTINCT path) FROM (SELECT ord, path, line FROM ({}) \
         ORDER BY ord, path, line LIMIT ?)",
        branches(scope, matched)
    )
}

/// Distinct files holding at least one observation of any mechanic in scope.
fn files_total_sql(scope: &[&Mechanic], matched: bool) -> String {
    format!(
        "SELECT count(DISTINCT path) FROM ({})",
        branches(scope, matched)
    )
}

/// One bound glob per branch, in branch order.
fn pattern_binds(glob: Option<&str>, branches: usize) -> Vec<Value> {
    match glob {
        Some(glob) => vec![Value::Text(glob.to_owned()); branches],
        None => Vec::new(),
    }
}

/// Reads one compact page over `scope`, in `rows_sql` order.
///
/// Identical paging contract to the full reader: the caller's `limit` and the
/// response byte budget cut the page, the first row is always returned even
/// when it alone exceeds `max_bytes`, and one row past `limit` is fetched only
/// to report `limited_by`.
pub(super) fn rows_for(
    tx: &Transaction<'_>,
    scope: &[&Mechanic],
    pattern: Option<&str>,
    limit: Option<usize>,
    offset: usize,
    max_bytes: usize,
) -> Result<Page<CompactLeadRow>, ErrorData> {
    let glob = compile_pattern(pattern);
    let fetch = limit.map(|rows| rows.saturating_add(1));
    let mut binds = pattern_binds(glob.as_deref(), scope.len());
    binds.push(Value::Integer(int_bind(fetch)?));
    binds.push(Value::Integer(int_bind(Some(offset))?));

    let mut statement = tx
        .prepare(&rows_sql(scope, glob.is_some()))
        .map_err(|error| super::context::internal_error("preparing compact leads rows", error))?;
    let mapped = statement
        .query_map(params_from_iter(binds), |row| {
            Ok(CompactLeadRow {
                mechanic: row.get(0)?,
                file: row.get(1)?,
                line: row.get(2)?,
            })
        })
        .map_err(|error| super::context::internal_error("reading compact leads rows", error))?;
    fill_page(mapped, limit, max_bytes, "reading compact leads rows")
}

/// Distinct files represented in the first `through` rows of the scope's
/// ordered result, across every mechanic in scope. `through = 0` is zero by
/// construction.
pub(super) fn files_covered(
    tx: &Transaction<'_>,
    scope: &[&Mechanic],
    pattern: Option<&str>,
    through: usize,
) -> Result<i64, ErrorData> {
    let glob = compile_pattern(pattern);
    let mut binds = pattern_binds(glob.as_deref(), scope.len());
    binds.push(Value::Integer(int_bind(Some(through))?));
    tx.query_row(
        &covered_sql(scope, glob.is_some()),
        params_from_iter(binds),
        |row| row.get(0),
    )
    .map_err(|error| super::context::internal_error("counting covered compact files", error))
}

/// Distinct files holding at least one observation of any mechanic in scope —
/// the denominator a whole-surface sweep can exhaust.
pub(super) fn files_total(
    tx: &Transaction<'_>,
    scope: &[&Mechanic],
    pattern: Option<&str>,
) -> Result<i64, ErrorData> {
    let glob = compile_pattern(pattern);
    let binds = pattern_binds(glob.as_deref(), scope.len());
    tx.query_row(
        &files_total_sql(scope, glob.is_some()),
        params_from_iter(binds),
        |row| row.get(0),
    )
    .map_err(|error| super::context::internal_error("counting compact files", error))
}
