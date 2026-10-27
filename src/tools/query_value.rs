// slugaudit-line-exception: approved-by=slugthug; reason=One responsibility: turning SQLite rows into the response rows (value caps, JSON conversion, store cells) and cutting them into a byte-budgeted page; the page builder and the conversions it uses must stay together.
//! Reads one SQLite row and converts it to JSON, enforcing the per-value byte
//! cap before a TEXT/BLOB value is JSON-cloned or hex-expanded. Kept separate
//! from `query` itself so that module stays focused on orchestrating
//! execution (budgets, response-size shrinking) rather than value framing.
//! The page-level result shapes ([`Collected`], [`PageBuilder`]) live here
//! too, beside the converted rows they describe.
use super::context::internal_error;
use super::query_clip::{ClippedCell, clip_row};
use crate::store::{Cell, NonFinite, StoredRow, Take};
use crate::util;
use rmcp::ErrorData;
use rusqlite::Row;
use rusqlite::types::{Type as SqlType, Value as SqlValue, ValueRef};
use schemars::JsonSchema;
use serde::Serialize;

/// One page of a result plus the facts that make it quotable.
pub(super) struct Collected {
    pub(super) rows: Vec<serde_json::Value>,
    /// Serialized JSON bytes of `rows`, measured per row.
    pub(super) bytes_used: usize,
    /// The cells cut from the first row because it alone exceeded the page.
    pub(super) clipped: Vec<ClippedCell>,
    /// The REAL cells that are `null` in `rows` only because JSON cannot carry
    /// them (see [`NonFiniteCell`]).
    pub(super) non_finite: Vec<NonFiniteCell>,
}

/// Cuts a stored result into one byte-budgeted page of JSON rows.
///
/// The store offers every row of the page to [`PageBuilder::accept`], the
/// first included. The first (always kept, so paging makes progress) is
/// clipped when it alone exceeds the page; later rows are taken while they
/// still fit. The column names and the budget are not known when the builder
/// is made (a new request learns the names from its prepared statement, a
/// continuation from the stored result), so they are set before the first
/// row is offered. An error while measuring a row cannot be returned from
/// `accept`, so it is kept and surfaces from [`PageBuilder::finish`].
pub(super) struct PageBuilder {
    names: Vec<String>,
    page_bytes: usize,
    used: usize,
    rows: Vec<serde_json::Value>,
    clipped: Vec<ClippedCell>,
    non_finite: Vec<NonFiniteCell>,
    failure: Option<ErrorData>,
}

impl PageBuilder {
    pub(super) fn new() -> Self {
        Self {
            names: Vec::new(),
            page_bytes: 0,
            used: 0,
            rows: Vec::new(),
            clipped: Vec::new(),
            non_finite: Vec::new(),
            failure: None,
        }
    }

    /// Sets the result's column names, before the first row is offered.
    pub(super) fn set_names(&mut self, names: Vec<String>) {
        self.names = names;
    }

    /// Sets the byte budget of the page, before the first row is offered.
    pub(super) fn set_budget(&mut self, page_bytes: usize) {
        self.page_bytes = page_bytes;
    }

    /// Takes the page's first row at result position `ord`.
    fn take_first(&mut self, values: &[SqlValue], ord: u64) -> Result<(), ErrorData> {
        let row_index = row_index_of(ord)?;
        // The bound never exceeds the exact serialized size, so a row above
        // the page is clipped without converting it first.
        let (row, bytes, cells) = if row_serialized_lower_bound(values) > self.page_bytes {
            self.clip(values, row_index)?
        } else {
            let (row, bytes) = json_row(&self.names, values)?;
            if bytes > self.page_bytes {
                self.clip(values, row_index)?
            } else {
                (row, bytes, Vec::new())
            }
        };
        self.used = bytes;
        self.clipped = cells;
        self.non_finite
            .extend(non_finite_cells(&self.names, values, row_index));
        self.rows.push(row);
        Ok(())
    }

    fn clip(
        &self,
        values: &[SqlValue],
        row_index: usize,
    ) -> Result<(serde_json::Value, usize, Vec<ClippedCell>), ErrorData> {
        let cut = clip_row(&self.names, values, self.page_bytes, row_index)?;
        let bytes = util::serialized_len(&cut.row)
            .map_err(|error| internal_error("measuring query row size", error))?;
        Ok((cut.row, bytes, cut.cells))
    }

    /// The store's question for each row of the page: does it still fit? The
    /// first row is always kept (the store ignores the verdict for it).
    pub(super) fn accept(&mut self, row: &StoredRow) -> Take {
        if self.failure.is_some() {
            return Take::No;
        }
        match self.try_accept(row) {
            Ok(take) => take,
            Err(error) => {
                self.failure = Some(error);
                Take::No
            }
        }
    }

    fn try_accept(&mut self, row: &StoredRow) -> Result<Take, ErrorData> {
        let values = values_of(row);
        if self.rows.is_empty() {
            self.take_first(&values, row.ord)?;
            return Ok(Take::Yes);
        }
        let row_index = row_index_of(row.ord)?;
        if row_serialized_lower_bound(&values) > self.page_bytes {
            return Ok(Take::No);
        }
        let (json, bytes) = json_row(&self.names, &values)?;
        if self.used.saturating_add(bytes) > self.page_bytes {
            return Ok(Take::No);
        }
        self.used = self.used.saturating_add(bytes);
        self.non_finite
            .extend(non_finite_cells(&self.names, &values, row_index));
        self.rows.push(json);
        Ok(Take::Yes)
    }

    /// The finished page. `served` is the number of rows the store returned,
    /// which must be the rows built here.
    ///
    /// # Errors
    ///
    /// The first measuring failure, or a disagreement with the store.
    pub(super) fn finish(self, served: usize) -> Result<Collected, ErrorData> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.rows.len() != served {
            return Err(internal_error(
                "building a query page",
                "the store returned a different number of rows than were measured",
            ));
        }
        Ok(Collected {
            rows: self.rows,
            bytes_used: self.used,
            clipped: self.clipped,
            non_finite: self.non_finite,
        })
    }
}

fn row_index_of(ord: u64) -> Result<usize, ErrorData> {
    usize::try_from(ord).map_err(|error| internal_error("reading a result row position", error))
}

/// One row as JSON and its serialized size.
fn json_row(
    column_names: &[String],
    values: &[SqlValue],
) -> Result<(serde_json::Value, usize), ErrorData> {
    let value = row_to_json(column_names, values)?;
    let bytes = util::serialized_len(&value)
        .map_err(|error| internal_error("measuring query row size", error))?;
    Ok((value, bytes))
}

/// The store cell for a SQLite value; the storage class is kept.
pub(super) fn cell_from(value: SqlValue) -> Cell {
    match value {
        SqlValue::Null => Cell::Null,
        SqlValue::Integer(number) => Cell::Int(number),
        SqlValue::Real(number) => Cell::Real(number),
        SqlValue::Text(text) => Cell::Text(text),
        SqlValue::Blob(bytes) => Cell::Blob(bytes),
    }
}

/// A stored row's values with its `nonfinite` marks put back: a NaN is stored
/// as NULL and marked, so the mark restores the REAL that
/// [`non_finite_cells`] reports.
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

/// One REAL cell whose value JSON cannot carry (an infinity; SQLite turns NaN
/// into NULL itself). JSON has no such number, so the cell is `null` in `rows`;
/// this entry says it was not SQL NULL and what it really was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NonFiniteCell {
    /// Zero-based index of the row in the whole result.
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
