//! Reads one SQLite row and converts it to JSON, enforcing the per-value byte
//! cap before a TEXT/BLOB value is JSON-cloned or hex-expanded. Kept separate
//! from `query` itself so that module stays focused on orchestrating
//! execution (budgets, response-size shrinking) rather than value framing.
use crate::util;
use rusqlite::Row;
use rusqlite::types::{Type as SqlType, Value as SqlValue};
use schemars::JsonSchema;
use serde::Serialize;

/// One REAL cell whose value JSON cannot carry (an infinity; SQLite turns NaN
/// into NULL itself). JSON has no such number, so the cell is `null` in `rows`;
/// this entry says it was not SQL NULL and what it really was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NonFiniteCell {
    /// Zero-based index of the row in the whole result (`offset` plus its
    /// position on the page), the same numbering `offset` uses.
    pub row: usize,
    /// Result column name of the cell.
    pub column: String,
    /// The real value: `"Infinity"`, `"-Infinity"` or `"NaN"`.
    pub value: &'static str,
}

/// The non-finite REAL cells of one row (see [`NonFiniteCell`]).
pub(super) fn non_finite_cells(
    column_names: &[String],
    values: &[SqlValue],
    row: usize,
) -> Vec<NonFiniteCell> {
    column_names
        .iter()
        .zip(values)
        .filter_map(|(column, value)| match value {
            SqlValue::Real(number) if !number.is_finite() => Some(NonFiniteCell {
                row,
                column: column.clone(),
                value: if number.is_nan() {
                    "NaN"
                } else if number.is_sign_negative() {
                    "-Infinity"
                } else {
                    "Infinity"
                },
            }),
            _ => None,
        })
        .collect()
}

/// Reads one row's cells, enforcing the per-value cap on each TEXT/BLOB.
pub(super) fn read_row(
    row: &Row<'_>,
    column_count: usize,
    value_cap: usize,
) -> rusqlite::Result<Vec<SqlValue>> {
    let mut values = Vec::with_capacity(column_count);
    for index in 0..column_count {
        let value: SqlValue = row.get(index)?;
        match &value {
            SqlValue::Text(text) => reject_oversized(text.len(), value_cap, index, SqlType::Text)?,
            SqlValue::Blob(bytes) => {
                reject_oversized(bytes.len(), value_cap, index, SqlType::Blob)?
            }
            _ => {}
        }
        values.push(value);
    }
    Ok(values)
}

/// One row as a JSON object, column names as keys, BLOBs hex-encoded.
pub(super) fn row_to_json(column_names: &[String], values: &[SqlValue]) -> serde_json::Value {
    let mut object = serde_json::Map::with_capacity(column_names.len());
    for (name, value) in column_names.iter().zip(values) {
        object.insert(name.clone(), sql_value_to_json(value));
    }
    serde_json::Value::Object(object)
}

/// A non-finite REAL becomes `null`; [`non_finite_cells`] reports it.
pub(super) fn sql_value_to_json(value: &SqlValue) -> serde_json::Value {
    match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(number) => serde_json::Value::from(*number),
        SqlValue::Real(number) => serde_json::Number::from_f64(*number)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        SqlValue::Text(text) => serde_json::Value::String(text.clone()),
        SqlValue::Blob(bytes) => serde_json::Value::String(util::hex_encode(bytes)),
    }
}

/// Fails closed on an oversized TEXT/BLOB value before it is cloned or
/// hex-expanded: checking the raw length up front avoids the memory spike a
/// huge value would otherwise cause, which the eventual response-size check
/// is too late to prevent.
fn reject_oversized(len: usize, cap: usize, index: usize, kind: SqlType) -> rusqlite::Result<()> {
    if len <= cap {
        return Ok(());
    }
    Err(rusqlite::Error::FromSqlConversionFailure(
        index,
        kind,
        Box::new(ValueTooLarge { len, cap }),
    ))
}

#[derive(Debug)]
struct ValueTooLarge {
    len: usize,
    cap: usize,
}

impl std::fmt::Display for ValueTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "value is {} bytes, exceeding the {}-byte per-value cap; select it in pieces \
             with substr(<column>, <start>, <length>) or line_range(<column>, <first>, <last>)",
            self.len, self.cap
        )
    }
}

impl std::error::Error for ValueTooLarge {}
