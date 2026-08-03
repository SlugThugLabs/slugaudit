//! Converts one SQLite row/value into JSON, enforcing the per-value byte
//! cap before a TEXT/BLOB value is cloned or hex-expanded. Kept separate
//! from `query` itself so that module stays focused on orchestrating
//! execution (budgets, response-size shrinking) rather than value framing.
use crate::util;
use rusqlite::Row;
use rusqlite::types::{Type as SqlType, Value as SqlValue};

pub(super) fn row_to_json(
    row: &Row,
    column_names: &[String],
    value_cap: usize,
) -> rusqlite::Result<serde_json::Value> {
    let mut object = serde_json::Map::with_capacity(column_names.len());
    for (index, name) in column_names.iter().enumerate() {
        let value: SqlValue = row.get(index)?;
        object.insert(name.clone(), sql_value_to_json(&value, index, value_cap)?);
    }
    Ok(serde_json::Value::Object(object))
}

fn sql_value_to_json(
    value: &SqlValue,
    index: usize,
    value_cap: usize,
) -> rusqlite::Result<serde_json::Value> {
    match value {
        SqlValue::Null => Ok(serde_json::Value::Null),
        SqlValue::Integer(number) => Ok(serde_json::Value::from(*number)),
        SqlValue::Real(number) => Ok(serde_json::Number::from_f64(*number)
            .map_or(serde_json::Value::Null, serde_json::Value::Number)),
        SqlValue::Text(text) => {
            reject_oversized(text.len(), value_cap, index, SqlType::Text)?;
            Ok(serde_json::Value::String(text.clone()))
        }
        SqlValue::Blob(bytes) => {
            reject_oversized(bytes.len(), value_cap, index, SqlType::Blob)?;
            Ok(serde_json::Value::String(util::hex_encode(bytes)))
        }
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
            "value is {} bytes, exceeding the {}-byte per-value cap",
            self.len, self.cap
        )
    }
}

impl std::error::Error for ValueTooLarge {}
