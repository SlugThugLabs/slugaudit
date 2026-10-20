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

/// One TEXT or BLOB cell that was cut to fit the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ClippedCell {
    /// Zero-based index of the row in the whole result (`offset` plus its
    /// position on the page), the same numbering `offset` uses.
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
struct Variable {
    index: usize,
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
    let room = page_bytes.saturating_sub(
        serialized_len(&object)
            .map_err(|error| internal_error("clipping an oversized query row", error))?,
    );
    let granted = share_room(&variables, room);

    let mut cells = Vec::new();
    for (variable, budget) in variables.iter().zip(granted) {
        let name = &column_names[variable.index];
        let Some((content, returned, kind, unit)) = clip_value(&values[variable.index], budget)
        else {
            continue;
        };
        if returned < variable.units {
            cells.push(clipped_cell(
                row_index,
                name,
                (kind, unit),
                returned,
                variable.units,
            ));
        }
        object.insert(name.clone(), serde_json::Value::String(content));
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
fn base_object(
    column_names: &[String],
    values: &[SqlValue],
) -> Result<(serde_json::Map<String, serde_json::Value>, Vec<Variable>), ErrorData> {
    ensure_unique_column_names(column_names)?;
    let mut object = serde_json::Map::with_capacity(column_names.len());
    let mut variables = Vec::new();
    for (index, (name, value)) in column_names.iter().zip(values).enumerate() {
        let cell = match value {
            SqlValue::Text(text) => {
                let (cost, chars) = text_size(text);
                let units = if text.contains('\0') {
                    text.len()
                } else {
                    chars
                };
                variables.push(Variable { index, cost, units });
                serde_json::Value::String(String::new())
            }
            SqlValue::Blob(bytes) => {
                variables.push(Variable {
                    index,
                    cost: bytes.len().saturating_mul(2),
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
fn share_room(variables: &[Variable], room: usize) -> Vec<usize> {
    let mut room = room;
    let mut order: Vec<usize> = (0..variables.len()).collect();
    order.sort_by_key(|position| variables[*position].cost);
    let mut granted = vec![0; variables.len()];
    for (rank, position) in order.iter().enumerate() {
        let share = room / (order.len() - rank);
        granted[*position] = variables[*position].cost.min(share);
        room -= granted[*position];
    }
    granted
}

/// Cuts one TEXT/BLOB cell to `budget` units, returning the content, how
/// many units were kept, and the kind/unit labels. Other cells yield `None`.
fn clip_value(
    value: &SqlValue,
    budget: usize,
) -> Option<(String, usize, &'static str, &'static str)> {
    match value {
        SqlValue::Text(text) => {
            let (prefix, chars) = text_prefix(text, budget);
            if text.contains('\0') {
                // SQLite's text functions stop at a NUL, so the rest is
                // reachable only as bytes of the value cast to a blob.
                Some((prefix.to_owned(), prefix.len(), "text", "bytes"))
            } else {
                Some((prefix.to_owned(), chars, "text", "characters"))
            }
        }
        SqlValue::Blob(bytes) => {
            let keep = (budget / 2).max(1).min(bytes.len());
            Some((hex_encode(&bytes[..keep]), keep, "blob", "bytes"))
        }
        _ => None,
    }
}

fn clipped_cell(
    row: usize,
    column: &str,
    (kind, unit): (&'static str, &'static str),
    returned: usize,
    total: usize,
) -> ClippedCell {
    let next_start = returned + 1;
    // The rest of the value, not just another chunk the size of the first: the
    // field promises the caller the remainder, and a chunk-sized continuation
    // would let an agent stop early and silently miss most of a large cell.
    let length = total - returned;
    let quoted = format!("\"{}\"", column.replace('"', "\"\""));
    // Text with a NUL is continued as bytes of the value cast to a blob.
    let target = if kind == "text" && unit == "bytes" {
        format!("CAST({quoted} AS BLOB)")
    } else {
        quoted
    };
    ClippedCell {
        row,
        column: column.to_owned(),
        kind,
        unit,
        returned,
        total,
        next_start,
        continue_with: format!("substr({target}, {next_start}, {length})"),
    }
}

/// Serialized JSON bytes a character takes inside a string.
fn json_char_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        control if control < ' ' => 6,
        other => other.len_utf8(),
    }
}

/// `(serialized content bytes, characters)` of a whole text.
fn text_size(text: &str) -> (usize, usize) {
    text.chars().fold((0, 0), |(cost, chars), character| {
        (cost + json_char_len(character), chars + 1)
    })
}

/// The longest prefix whose serialized content fits `budget` (at least one
/// character), and its length in characters.
fn text_prefix(text: &str, budget: usize) -> (&str, usize) {
    let mut used = 0;
    let mut chars = 0;
    let mut end = 0;
    for (position, character) in text.char_indices() {
        let next = used + json_char_len(character);
        if chars > 0 && next > budget {
            break;
        }
        used = next;
        chars += 1;
        end = position + character.len_utf8();
    }
    (&text[..end], chars)
}
