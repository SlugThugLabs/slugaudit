//! The value model of a stored row: the five SQLite storage classes, the
//! non-finite marks, and the typed mapping trait.

use super::error::ResultStoreError;
use rusqlite::types::{ToSql, ToSqlOutput, ValueRef};

/// One stored value, keeping its exact SQLite storage class.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// SQL NULL.
    Null,
    /// 64-bit integer.
    Int(i64),
    /// IEEE double. A NaN is stored as NULL and marked in `nonfinite`.
    Real(f64),
    /// UTF-8 text, embedded NULs included.
    Text(String),
    /// Raw bytes; empty stays a BLOB.
    Blob(Vec<u8>),
}

impl Cell {
    /// Payload size, used to decide when to measure the file while writing.
    pub(super) fn payload_len(&self) -> usize {
        match self {
            Self::Null | Self::Int(_) | Self::Real(_) => 8,
            Self::Text(text) => text.len(),
            Self::Blob(bytes) => bytes.len(),
        }
    }
}

impl ToSql for Cell {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Int(value) => ValueRef::Integer(*value),
            Self::Real(value) => ValueRef::Real(*value),
            Self::Text(text) => ValueRef::Text(text.as_bytes()),
            Self::Blob(bytes) => ValueRef::Blob(bytes),
        }))
    }
}

/// A value SQLite cannot round-trip or JSON cannot carry, marked per cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonFinite {
    /// Stored as NULL (SQLite has no NaN).
    NaN,
    /// Stored natively and marked.
    Infinity,
    /// Stored natively and marked.
    NegInfinity,
}

impl NonFinite {
    /// The mark for a double, or `None` when it is finite.
    pub(super) fn of(value: f64) -> Option<Self> {
        if value.is_nan() {
            Some(Self::NaN)
        } else if value == f64::INFINITY {
            Some(Self::Infinity)
        } else if value == f64::NEG_INFINITY {
            Some(Self::NegInfinity)
        } else {
            None
        }
    }

    /// The name stored in the `nonfinite` JSON.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::NaN => "NaN",
            Self::Infinity => "Infinity",
            Self::NegInfinity => "-Infinity",
        }
    }

    pub(super) fn parse(name: &str) -> Option<Self> {
        match name {
            "NaN" => Some(Self::NaN),
            "Infinity" => Some(Self::Infinity),
            "-Infinity" => Some(Self::NegInfinity),
            _ => None,
        }
    }
}

/// One row as read back.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredRow {
    /// Position in the result, from 0.
    pub ord: u64,
    /// The cells in column order (a NaN reads as `Null`).
    pub cells: Vec<Cell>,
    /// `(column index, mark)` for every non-finite cell; the tool layer
    /// decides what to emit.
    pub nonfinite: Vec<(usize, NonFinite)>,
}

/// A caller's verdict on one row while a page is assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// Include the row.
    Yes,
    /// Stop before the row; it starts the next page.
    No,
}

/// A row type with a fixed column layout.
pub trait Materialize: Sized {
    /// Column names, stored in `meta` only.
    fn columns() -> &'static [&'static str];
    /// The cells in `columns()` order.
    fn to_values(&self) -> Vec<Cell>;
    /// Rebuilds the row from stored cells.
    ///
    /// # Errors
    ///
    /// Fails when the cells do not match the layout.
    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError>;
}
