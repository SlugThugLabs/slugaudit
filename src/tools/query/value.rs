//! SQLite row reading, per-value byte cap enforcement, `StoredRow` <-> `SqlValue`
//! conversion, and JSON row encoding for `query`.

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::store::{Cell, NonFinite, StoredRow};
use crate::util;
use rmcp::ErrorData;
use rusqlite::Row;
use rusqlite::types::{Type as SqlType, Value as SqlValue, ValueRef};
use schemars::JsonSchema;
use serde::Serialize;

pub(super) fn cell_from(value: SqlValue) -> Cell {
    match value {
        SqlValue::Null => Cell::Null,
        SqlValue::Integer(number) => Cell::Int(number),
        SqlValue::Real(number) => Cell::Real(number),
        SqlValue::Text(text) => Cell::Text(text),
        SqlValue::Blob(bytes) => Cell::Blob(bytes),
    }
}

pub(super) fn values_of(row: &StoredRow) -> Vec<SqlValue> {
    row.cells
        .iter()
        .enumerate()
        .map(|(index, cell)| match cell {
            Cell::Null if is_marked_nan(row, index) => SqlValue::Real(f64::NAN),
            Cell::Null => SqlValue::Null,
            Cell::Int(number) => SqlValue::Integer(*number),
            Cell::Real(number) => SqlValue::Real(*number),
            Cell::Text(text) => SqlValue::Text(text.clone()),
            Cell::Blob(bytes) => SqlValue::Blob(bytes.clone()),
        })
        .collect()
}

fn is_marked_nan(row: &StoredRow, index: usize) -> bool {
    row.nonfinite
        .iter()
        .any(|(column, mark)| *column == index && *mark == NonFinite::NaN)
}

/// One REAL cell whose value JSON cannot carry (`Infinity`, `-Infinity`, `NaN`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NonFiniteCell {
    /// Zero-based index of the row in the whole result.
    pub row: usize,
    /// Result column name of the cell.
    pub column: String,
    /// The real value: `"Infinity"`, `"-Infinity"` or `"NaN"`.
    pub value: &'static str,
}

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

pub(super) fn read_row(
    row: &Row<'_>,
    column_count: usize,
    value_cap: usize,
) -> rusqlite::Result<Vec<SqlValue>> {
    let mut values = Vec::with_capacity(column_count);
    for index in 0..column_count {
        match row.get_ref(index)? {
            ValueRef::Text(bytes) => {
                reject_oversized(bytes.len(), value_cap, index, SqlType::Text)?;
            }
            ValueRef::Blob(bytes) => {
                reject_oversized(bytes.len(), value_cap, index, SqlType::Blob)?;
            }
            ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => {}
        }
        values.push(row.get(index)?);
    }
    Ok(values)
}

pub(super) fn ensure_unique_column_names(column_names: &[String]) -> Result<(), ErrorData> {
    let mut seen = std::collections::HashSet::with_capacity(column_names.len());
    for name in column_names {
        if !seen.insert(name.as_str()) {
            return Err(invalid_params_with_code(
                format!("duplicate column name {name:?} in query result; alias the columns"),
                ErrorCode::InvalidInput,
            ));
        }
    }
    Ok(())
}

pub(super) fn build_row_object(
    column_names: &[String],
    values: impl IntoIterator<Item = serde_json::Value>,
) -> serde_json::Value {
    let mut object = serde_json::Map::with_capacity(column_names.len());
    for (name, json) in column_names.iter().zip(values) {
        object.insert(name.clone(), json);
    }
    serde_json::Value::Object(object)
}

#[cfg(test)]
pub(super) fn row_to_json(
    column_names: &[String],
    values: &[SqlValue],
) -> Result<serde_json::Value, ErrorData> {
    ensure_unique_column_names(column_names)?;
    Ok(build_row_object(
        column_names,
        values.iter().map(sql_value_to_json),
    ))
}

pub(super) fn into_json_value(value: SqlValue) -> serde_json::Value {
    match value {
        SqlValue::Text(text) => serde_json::Value::String(text),
        other => sql_value_to_json(&other),
    }
}

pub(super) fn sql_value_to_json(value: &SqlValue) -> serde_json::Value {
    match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(number) => serde_json::Value::from(*number),
        SqlValue::Real(number) => match serde_json::Number::from_f64(*number) {
            Some(finite) => serde_json::Value::Number(finite),
            None => serde_json::Value::Null,
        },
        SqlValue::Text(text) => serde_json::Value::String(text.clone()),
        SqlValue::Blob(bytes) => serde_json::Value::String(util::hex_encode(bytes)),
    }
}

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
