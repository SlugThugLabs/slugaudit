//! Reads one SQLite row and converts it to JSON, enforcing the per-value byte
//! cap before a TEXT/BLOB value is JSON-cloned or hex-expanded. Kept separate
//! from `query` itself so that module stays focused on orchestrating
//! execution (budgets, response-size shrinking) rather than value framing.
//! The page-level result shapes ([`Collected`], [`RowTotal`]) live here too,
//! beside the converted rows they describe.
use super::query_clip::ClippedCell;
use crate::util;
use rmcp::ErrorData;
use rusqlite::Row;
use rusqlite::types::{Type as SqlType, Value as SqlValue, ValueRef};
use schemars::JsonSchema;
use serde::Serialize;

/// The size of the whole result, independent of the page that was returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowTotal {
    /// Exact row count of the whole result.
    Exact(i64),
    /// Counting did not finish; the static reason names the budget that
    /// stopped it. No estimate is ever substituted.
    Unavailable(&'static str),
}

/// One page of a result plus the facts that make it quotable.
pub(super) struct Collected {
    pub(super) rows: Vec<serde_json::Value>,
    /// True when more rows remain past this page.
    pub(super) truncated: bool,
    /// Serialized JSON bytes of `rows`, measured per row.
    pub(super) rows_bytes: usize,
    pub(super) total: RowTotal,
    /// The cells cut from the first row because it alone exceeded the page.
    pub(super) clipped: Vec<ClippedCell>,
    /// The REAL cells that are `null` in `rows` only because JSON cannot carry
    /// them (see [`NonFiniteCell`]).
    pub(super) non_finite: Vec<NonFiniteCell>,
}

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
        // `get_ref` borrows SQLite's buffer, so an oversized value is rejected
        // before `get` copies it into an owned `Value` that would be dropped
        // unread: the cap check never allocates the bytes it refuses.
        match row.get_ref(index)? {
            ValueRef::Text(bytes) => {
                reject_oversized(bytes.len(), value_cap, index, SqlType::Text)?
            }
            ValueRef::Blob(bytes) => {
                reject_oversized(bytes.len(), value_cap, index, SqlType::Blob)?
            }
            ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => {}
        }
        values.push(row.get(index)?);
    }
    Ok(values)
}

/// A lower bound on the serialized JSON size of one row, computed from the raw
/// cells without converting them.
///
/// Object punctuation, column names and the quotes around string values are
/// all omitted, as are NULL/integer/real contributions; a TEXT cell counts its
/// UTF-8 byte length (JSON escaping only ever adds bytes) and a BLOB cell
/// counts its hex length. Every omitted term is positive, so this never
/// exceeds the exact `serialized_len(&row_to_json(..))`: a row whose bound
/// already exceeds the page can be dropped without converting it.
pub(super) fn row_serialized_lower_bound(values: &[SqlValue]) -> usize {
    values.iter().fold(0usize, |total, value| {
        let contribution = match value {
            SqlValue::Text(text) => text.len(),
            SqlValue::Blob(bytes) => bytes.len().saturating_mul(2),
            SqlValue::Null | SqlValue::Integer(_) | SqlValue::Real(_) => 0,
        };
        total.saturating_add(contribution)
    })
}

/// Rejects a result whose column names are not unique.
///
/// Both row builders ([`row_to_json`] and `query_clip::base_object`) key a JSON
/// object by column name, so a repeated name would silently keep one column
/// and drop the other. Called from each builder so neither path can diverge.
pub(super) fn ensure_unique_column_names(column_names: &[String]) -> Result<(), ErrorData> {
    let mut seen = std::collections::HashSet::with_capacity(column_names.len());
    for name in column_names {
        if !seen.insert(name.as_str()) {
            return Err(ErrorData::invalid_params(
                format!("duplicate column name {name:?} in query result; alias the columns"),
                None,
            ));
        }
    }
    Ok(())
}

/// One row as a JSON object, column names as keys, BLOBs hex-encoded.
///
/// # Errors
///
/// Returns `invalid_params` when a column name repeats: the object cannot carry
/// both columns, and keeping only one would be a wrong answer, so the caller is
/// told to alias the columns instead.
pub(super) fn row_to_json(
    column_names: &[String],
    values: &[SqlValue],
) -> Result<serde_json::Value, ErrorData> {
    ensure_unique_column_names(column_names)?;
    let mut object = serde_json::Map::with_capacity(column_names.len());
    for (name, value) in column_names.iter().zip(values) {
        object.insert(name.clone(), sql_value_to_json(value));
    }
    Ok(serde_json::Value::Object(object))
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
