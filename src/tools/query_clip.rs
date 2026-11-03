//! Clipping the oversized first row of a `query` page.
//!
//! A row is never split across pages, so a row whose serialized size alone
//! exceeds the page (typically one `files.content` cell, or a `group_concat`)
//! would otherwise go out whole. Instead its TEXT/BLOB cells are cut to fit,
//! and every cut is reported as a [`ClippedCell`] carrying the exact
//! continuation, so nothing is lost and nothing is silent.
//!
//! Units are SQLite's `substr` units, so the reported numbers feed `substr`
//! directly: characters for TEXT (never splitting a character), bytes for BLOB
//! (the JSON value is hex, two characters per byte). Budgets are measured in
//! serialized JSON bytes — string escapes and BLOB hex included.

use super::context::internal_error;
use super::query_value::{ensure_unique_column_names, sql_value_to_json};
use crate::util::{hex_encode, serialized_len};
use rmcp::ErrorData;
use rusqlite::types::Value as SqlValue;
use schemars::JsonSchema;
use serde::Serialize;
use text::{text_prefix, text_size};

/// One TEXT or BLOB cell that was cut to fit the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ClippedCell {
    /// Zero-based index of the row in the whole result.
    pub row: usize,
    /// Result column name of the clipped cell.
    pub column: String,
    /// `"text"` or `"blob"`.
    pub kind: &'static str,
    /// What `returned`, `total` and `next_start` count, in `substr`'s own
    /// units: `"characters"` for text, `"bytes"` for a blob, and `"bytes"` for
    /// a text containing a NUL (SQLite's text functions stop at a NUL, so such
    /// a value is continued as bytes: `continue_with` casts it to a blob and
    /// the rest comes back hex-encoded).
    pub unit: &'static str,
    /// How many units of the value the response carries (from the start).
    pub returned: usize,
    /// How many units the whole value has: SQLite's `length(value)`.
    pub total: usize,
    /// 1-based position of the first unit not returned, ready for `substr`.
    pub next_start: usize,
    /// A ready-to-use expression for the rest of the value,
    /// `substr("<column>", start, length)`: running it returns every remaining
    /// unit, not another page-sized chunk. The column name is quoted as an
    /// identifier; apply the same `substr` to the expression that produced the
    /// column. If that continuation row is itself larger than the page, its
    /// response's `clipped_cells` entry carries the next expression, and so on
    /// until the value is complete.
    pub continue_with: String,
}

/// The clipped row and what was cut from it.
pub(super) struct ClippedRow {
    pub(super) row: serde_json::Value,
    pub(super) cells: Vec<ClippedCell>,
}

/// A TEXT/BLOB cell of the row, measured.
struct Variable<'a> {
    /// Result column name of the cell.
    name: &'a str,
    /// The cell itself.
    value: &'a SqlValue,
    /// Serialized JSON bytes of the value's content, without its quotes.
    cost: usize,
    /// Length in `substr` units.
    units: usize,
}

/// Cuts the TEXT/BLOB cells of `values` so the serialized row fits
/// `page_bytes`, sharing the room left after the fixed parts as evenly as the
/// smaller cells allow. A cell is cut by at least one unit of progress even
/// when `page_bytes` is smaller than a character, so paging by the hint
/// always advances.
///
/// # Errors
///
/// Returns `invalid_params` when a column name repeats (see
/// [`ensure_unique_column_names`]); the row cannot carry both columns.
/// Returns `internal_error` if the base row cannot be measured.
pub(super) fn clip_row(
    column_names: &[String],
    values: &[SqlValue],
    page_bytes: usize,
    row_index: usize,
) -> Result<ClippedRow, ErrorData> {
    let (mut object, variables) = base_object(column_names, values)?;
    // KEEP saturating: a base row that alone exceeds the page leaves no room for the clipped cells, and zero is exactly that room
    let room = page_bytes.saturating_sub(
        serialized_len(&object)
            .map_err(|error| internal_error("clipping an oversized query row", error))?,
    );
    let granted = share_room(&variables, room)?;

    let mut cells = Vec::new();
    for (variable, budget) in variables.iter().zip(granted) {
        let name = variable.name;
        let Some((content, returned, kind, unit)) = clip_value(variable.value, budget)? else {
            continue;
        };
        if returned < variable.units {
            cells.push(clipped_cell(
                row_index,
                name,
                (kind, unit),
                returned,
                variable.units,
            )?);
        }
        object.insert(name.to_owned(), serde_json::Value::String(content));
    }
    Ok(ClippedRow {
        row: serde_json::Value::Object(object),
        cells,
    })
}

/// The row with every TEXT/BLOB cell replaced by an empty string, plus the
/// measured variables those cells will be clipped from. Shares the duplicate
/// column check with [`super::query_value::row_to_json`] so the clipped and
/// unclipped paths cannot disagree on which results are representable.
fn base_object<'a>(
    column_names: &'a [String],
    values: &'a [SqlValue],
) -> Result<
    (
        serde_json::Map<String, serde_json::Value>,
        Vec<Variable<'a>>,
    ),
    ErrorData,
> {
    ensure_unique_column_names(column_names)?;
    let mut object = serde_json::Map::with_capacity(column_names.len());
    let mut variables = Vec::new();
    for (name, value) in column_names.iter().zip(values) {
        let cell = match value {
            SqlValue::Text(text) => {
                let (cost, chars) = text_size(text)?;
                let units = if text.contains('\0') {
                    text.len()
                } else {
                    chars
                };
                variables.push(Variable {
                    name,
                    value,
                    cost,
                    units,
                });
                serde_json::Value::String(String::new())
            }
            SqlValue::Blob(bytes) => {
                variables.push(Variable {
                    name,
                    value,
                    cost: bytes.len().checked_mul(2).ok_or_else(|| {
                        internal_error(
                            "clipping an oversized query row",
                            "a blob's hex length overflowed",
                        )
                    })?,
                    units: bytes.len(),
                });
                serde_json::Value::String(String::new())
            }
            other => sql_value_to_json(other),
        };
        object.insert(name.clone(), cell);
    }
    Ok((object, variables))
}

/// Shares `room` among the variables as evenly as the smaller cells allow,
/// smallest first, returning each variable's granted budget by index.
fn share_room(variables: &[Variable<'_>], room: usize) -> Result<Vec<usize>, ErrorData> {
    let mut room = room;
    let mut order: Vec<(usize, usize)> = variables
        .iter()
        .enumerate()
        .map(|(position, variable)| (position, variable.cost))
        .collect();
    order.sort_by_key(|(_, cost)| *cost);
    let mut grants = Vec::with_capacity(order.len());
    // The divisor is the number of cells not yet granted: it counts down from
    // the number of cells to 1, so the smallest cells are granted first.
    for ((position, cost), divisor) in order.into_iter().zip((1..=variables.len()).rev()) {
        let share = room
            .checked_div(divisor)
            .ok_or_else(|| room_error("dividing the room among cells"))?;
        let grant = cost.min(share);
        room = room
            .checked_sub(grant)
            .ok_or_else(|| room_error("granting a cell its room"))?;
        grants.push((position, grant));
    }
    grants.sort_by_key(|(position, _)| *position);
    Ok(grants.into_iter().map(|(_, grant)| grant).collect())
}

/// The error for an arithmetic step of clipping that failed.
fn room_error(step: &str) -> ErrorData {
    internal_error(
        "clipping an oversized query row",
        format!("{step} overflowed"),
    )
}

/// Cuts one TEXT/BLOB cell to `budget` units, returning the content, how
/// many units were kept, and the kind/unit labels. Other cells yield `None`.
fn clip_value(
    value: &SqlValue,
    budget: usize,
) -> Result<Option<(String, usize, &'static str, &'static str)>, ErrorData> {
    Ok(match value {
        SqlValue::Text(text) => {
            let (prefix, chars) = text_prefix(text, budget)?;
            if text.contains('\0') {
                // SQLite's text functions stop at a NUL, so the rest is
                // reachable only as bytes of the value cast to a blob.
                let bytes = prefix.len();
                Some((prefix, bytes, "text", "bytes"))
            } else {
                Some((prefix, chars, "text", "characters"))
            }
        }
        SqlValue::Blob(bytes) => {
            // A budget past the blob keeps the whole blob.
            let kept = bytes.split_at((budget / 2).max(1).min(bytes.len())).0;
            Some((hex_encode(kept), kept.len(), "blob", "bytes"))
        }
        _ => None,
    })
}

fn clipped_cell(
    row: usize,
    column: &str,
    (kind, unit): (&'static str, &'static str),
    returned: usize,
    total: usize,
) -> Result<ClippedCell, ErrorData> {
    let next_start = returned
        .checked_add(1)
        .ok_or_else(|| room_error("numbering the continuation"))?;
    // The rest of the value, not just another chunk the size of the first: the
    // field promises the caller the remainder, and a chunk-sized continuation
    // would let an agent stop early and silently miss most of a large cell.
    let length = total
        .checked_sub(returned)
        .ok_or_else(|| room_error("measuring the rest of the cell"))?;
    let quoted = format!("\"{}\"", column.replace('"', "\"\""));
    // Text with a NUL is continued as bytes of the value cast to a blob.
    let target = if kind == "text" && unit == "bytes" {
        format!("CAST({quoted} AS BLOB)")
    } else {
        quoted
    };
    Ok(ClippedCell {
        row,
        column: column.to_owned(),
        kind,
        unit,
        returned,
        total,
        next_start,
        continue_with: format!("substr({target}, {next_start}, {length})"),
    })
}

#[path = "query_clip_text.rs"]
mod text;
