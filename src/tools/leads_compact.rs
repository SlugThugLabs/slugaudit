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
//! That is why the read here walks one `UNION ALL` over the selected
//! mechanics rather than concatenating per-mechanic reads in Rust: the row
//! order (`ord`, then `path`, then `line`, then the row id) is defined by the
//! statement, so the stored result and `files_covered` agree without a second
//! ordering to keep in sync. `ord` is the position in `MECHANICS`; the
//! mechanic id itself is spliced as a literal from the static catalogue
//! (never from caller input), which is what keeps each branch self-identifying
//! without a lookup table in the response.
//!
//! The caller's `pattern` glob is bound as a parameter, exactly as in
//! `leads_query.rs`.

use super::leads::CompactLeadRow;
use super::leads_mechanics::{MECHANICS, Mechanic};
use super::leads_query::compile_pattern;
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

/// One `SELECT` per mechanic, each tagging its rows with the mechanic id, its
/// catalogue position and its row id. The id is a compile-time constant from
/// [`MECHANICS`], never a caller value.
fn branches(scope: &[&Mechanic], matched: bool) -> String {
    scope
        .iter()
        .enumerate()
        .map(|(ordinal, mechanic)| {
            format!(
                "SELECT {ordinal} AS ord, '{}' AS mechanic, {} AS path, {} AS line, \
                 {} AS rid FROM {}{}",
                mechanic.id,
                mechanic.path_expr(),
                mechanic.line_expr(),
                mechanic.id_expr(),
                mechanic.read_from(),
                mechanic.where_clause(matched)
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ")
}

/// The triple query: catalogue order, then the mechanic's own file/line
/// order, then the row id so equal lines keep one order. One pattern glob is
/// bound per branch.
fn rows_sql(scope: &[&Mechanic], matched: bool) -> String {
    format!(
        "SELECT mechanic, path, line FROM ({}) ORDER BY ord, path, line, rid",
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

/// Streams every compact row of `scope` under `pattern`, in `rows_sql` order,
/// into `on_row`.
pub(super) fn stream(
    tx: &Transaction<'_>,
    scope: &[&Mechanic],
    pattern: Option<&str>,
    on_row: &mut dyn FnMut(CompactLeadRow) -> Result<(), ErrorData>,
) -> Result<(), ErrorData> {
    let glob = compile_pattern(pattern);
    let binds = pattern_binds(glob.as_deref(), scope.len());
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
    for row in mapped {
        on_row(row.map_err(|error| {
            super::context::internal_error("reading compact leads rows", error)
        })?)?;
    }
    Ok(())
}
