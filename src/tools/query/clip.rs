//! Clipping the oversized first row of a `query` page.

use super::json_text::{text_prefix, text_size};
use super::value::{ensure_unique_column_names, sql_value_to_json};
use crate::tools::context::internal_error;
use crate::util::{hex_encode, serialized_len};
use rmcp::ErrorData;
use rusqlite::types::Value as SqlValue;
use schemars::JsonSchema;
use serde::Serialize;

/// One TEXT or BLOB cell that was cut to fit the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ClippedCell {
    /// Zero-based index of the row in the whole result.
    pub row: usize,
    /// Result column name of the clipped cell.
    pub column: String,
    /// `"text"` or `"blob"`.
    pub kind: &'static str,
    /// `"characters"` for text, `"bytes"` for a blob or NUL-containing text.
    pub unit: &'static str,
    /// Units of the value carried in this response.
    pub returned: usize,
    /// Total units in the full value.
    pub total: usize,
    /// 1-based position of the first unreturned unit.
    pub next_start: usize,
    /// Ready-to-use `substr(...)` continuation expression.
    pub continue_with: String,
}

pub(super) struct ClippedRow {
    pub(super) row: serde_json::Value,
    pub(super) cells: Vec<ClippedCell>,
}

#[derive(Clone, Copy)]
enum ClipTarget<'a> {
    Text { text: &'a str, has_nul: bool },
    Blob(&'a [u8]),
}

impl ClipTarget<'_> {
    const fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Text { has_nul: true, .. } => ("text", "bytes"),
            Self::Text { has_nul: false, .. } => ("text", "characters"),
            Self::Blob(_) => ("blob", "bytes"),
        }
    }

    fn sql_expr(self, column: &str) -> String {
        let quoted = format!("\"{}\"", column.replace('"', "\"\""));
        if matches!(self, Self::Text { has_nul: true, .. }) {
            format!("CAST({quoted} AS BLOB)")
        } else {
            quoted
        }
    }

    fn clip(self, budget: usize) -> Result<(String, usize), ErrorData> {
        match self {
            Self::Text { text, has_nul } => {
                let (prefix, chars) = text_prefix(text, budget).map_err(room_error)?;
                let returned = if has_nul { prefix.len() } else { chars };
                Ok((prefix, returned))
            }
            Self::Blob(bytes) => {
                let kept = bytes.split_at((budget / 2).max(1).min(bytes.len())).0;
                Ok((hex_encode(kept), kept.len()))
            }
        }
    }
}

struct Variable<'a> {
    name: &'a str,
    target: ClipTarget<'a>,
    cost: usize,
    units: usize,
}

impl<'a> Variable<'a> {
    fn from_sql(
        name: &'a str,
        value: &'a SqlValue,
    ) -> Result<Result<Self, serde_json::Value>, ErrorData> {
        match value {
            SqlValue::Text(text) => {
                let (cost, chars) = text_size(text).map_err(room_error)?;
                let has_nul = text.contains('\0');
                Ok(Ok(Self {
                    name,
                    target: ClipTarget::Text { text, has_nul },
                    cost,
                    units: if has_nul { text.len() } else { chars },
                }))
            }
            SqlValue::Blob(bytes) => {
                let cost = bytes
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| room_error("measuring a blob cell"))?;
                Ok(Ok(Self {
                    name,
                    target: ClipTarget::Blob(bytes),
                    cost,
                    units: bytes.len(),
                }))
            }
            other => Ok(Err(sql_value_to_json(other))),
        }
    }
}

pub(super) fn clip_row(
    column_names: &[String],
    values: &[SqlValue],
    page_bytes: usize,
    row_index: usize,
) -> Result<ClippedRow, ErrorData> {
    let (mut object, variables) = base_object(column_names, values)?;
    // KEEP saturating: a base row that alone exceeds the page leaves zero room for clipped cells
    let room = page_bytes.saturating_sub(
        serialized_len(&object)
            .map_err(|error| internal_error("clipping an oversized query row", error))?,
    );
    let costs: Vec<usize> = variables.iter().map(|var| var.cost).collect();
    let granted = share_room(&costs, room)?;

    let mut cells = Vec::new();
    for (var, budget) in variables.iter().zip(granted) {
        let (content, returned) = var.target.clip(budget)?;
        if returned < var.units {
            cells.push(clipped_cell(row_index, var, returned)?);
        }
        object.insert(var.name.to_owned(), serde_json::Value::String(content));
    }
    Ok(ClippedRow {
        row: serde_json::Value::Object(object),
        cells,
    })
}

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
        let cell = match Variable::from_sql(name, value)? {
            Ok(var) => {
                variables.push(var);
                serde_json::Value::String(String::new())
            }
            Err(scalar) => scalar,
        };
        object.insert(name.clone(), cell);
    }
    Ok((object, variables))
}

fn share_room(costs: &[usize], mut room: usize) -> Result<Vec<usize>, ErrorData> {
    let mut order: Vec<(usize, usize)> = costs.iter().copied().enumerate().collect();
    order.sort_by_key(|(_, cost)| *cost);
    let mut grants = Vec::with_capacity(order.len());
    for ((pos, cost), divisor) in order.into_iter().zip((1..=costs.len()).rev()) {
        let share = room
            .checked_div(divisor)
            .ok_or_else(|| room_error("dividing the room among cells"))?;
        let grant = cost.min(share);
        room = room
            .checked_sub(grant)
            .ok_or_else(|| room_error("granting a cell its room"))?;
        grants.push((pos, grant));
    }
    grants.sort_by_key(|(pos, _)| *pos);
    Ok(grants.into_iter().map(|(_, grant)| grant).collect())
}

fn room_error(step: &str) -> ErrorData {
    internal_error(
        "clipping an oversized query row",
        format!("{step} overflowed"),
    )
}

fn clipped_cell(row: usize, var: &Variable<'_>, returned: usize) -> Result<ClippedCell, ErrorData> {
    let next_start = returned
        .checked_add(1)
        .ok_or_else(|| room_error("numbering the continuation"))?;
    let length = var
        .units
        .checked_sub(returned)
        .ok_or_else(|| room_error("measuring the rest of the cell"))?;
    let (kind, unit) = var.target.labels();
    let target = var.target.sql_expr(var.name);
    Ok(ClippedCell {
        row,
        column: var.name.to_owned(),
        kind,
        unit,
        returned,
        total: var.units,
        next_start,
        continue_with: format!("substr({target}, {next_start}, {length})"),
    })
}
