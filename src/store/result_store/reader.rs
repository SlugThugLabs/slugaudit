//! Reading a page of a stored result. The reader holds no position: the same
//! `from_ord` always returns the same rows.

use super::cell::{Cell, Materialize, NonFinite, StoredRow, Take};
use super::cursor::{Cursor, ResultId};
use super::error::ResultStoreError;
use super::registry::ResultMeta;
use super::result_file::column_list;
use super::store::ResultStore;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use std::sync::Arc;

/// One page of a result.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRead {
    /// The rows the caller accepted, in ordinal order.
    pub rows: Vec<StoredRow>,
    /// Where the next page starts; `None` when this page reached the end.
    pub next_ord: Option<u64>,
    /// Exact number of rows in the result.
    pub row_count: u64,
    /// What the result is.
    pub meta: Arc<ResultMeta>,
}

/// A page of typed rows.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedPage<T> {
    /// The rows.
    pub rows: Vec<T>,
    /// Where the next page starts; `None` at the end.
    pub next_ord: Option<u64>,
    /// Exact number of rows in the result.
    pub row_count: u64,
    /// What the result is.
    pub meta: Arc<ResultMeta>,
}

impl ResultStore {
    /// Reads the page that starts at `from_ord`. The first row is always
    /// taken (the closure is not asked about it); each later row is offered to
    /// `accept` in ordinal order and the page ends at the first `No`. Serving
    /// a page renews the result's protection; serving the last one marks it
    /// delivered.
    ///
    /// # Errors
    ///
    /// `Expired` when the result was reclaimed, `Unknown` when `from_ord` is
    /// past the row count or the result is not complete, `Unavailable` or
    /// `Sqlite` for read failures.
    pub fn read_page(
        &self,
        id: ResultId,
        from_ord: u64,
        mut accept: impl FnMut(&StoredRow) -> Take,
    ) -> Result<PageRead, ResultStoreError> {
        let entry = self.lookup(id)?;
        if from_ord > entry.meta.row_count {
            return Err(self.unknown_cursor());
        }
        let (rows, next_ord) = {
            let conn = crate::util::lock_or_recover(&entry.conn, "result_entry_connection");
            if !is_complete(&conn)? {
                return Err(self.unknown_cursor());
            }
            read_rows(&conn, entry.meta.columns.len(), from_ord, &mut accept)?
        };
        entry.served(self.now(), next_ord.is_none());
        Ok(PageRead {
            rows,
            next_ord,
            row_count: entry.meta.row_count,
            meta: Arc::clone(&entry.meta),
        })
    }

    /// Decodes `token` and reads the page it names.
    ///
    /// # Errors
    ///
    /// As [`ResultStore::decode_cursor`] and [`ResultStore::read_page`].
    pub fn read_cursor(
        &self,
        token: &str,
        accept: impl FnMut(&StoredRow) -> Take,
    ) -> Result<PageRead, ResultStoreError> {
        let cursor = self.decode_cursor(token)?;
        self.read_page(cursor.result, cursor.ord, accept)
    }

    /// [`ResultStore::read_page`] over a [`Materialize`] row type.
    ///
    /// # Errors
    ///
    /// Those of `read_page`, plus whatever `T::from_values` reports.
    pub fn read_typed<T: Materialize>(
        &self,
        id: ResultId,
        from_ord: u64,
        mut accept: impl FnMut(&T) -> Take,
    ) -> Result<TypedPage<T>, ResultStoreError> {
        let mut failure = None;
        let page = self.read_page(id, from_ord, |row| match T::from_values(&row.cells) {
            Ok(typed) => accept(&typed),
            Err(error) => {
                failure.get_or_insert(error);
                Take::No
            }
        })?;
        if let Some(error) = failure {
            return Err(error);
        }
        let rows = page
            .rows
            .iter()
            .map(|row| T::from_values(&row.cells))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TypedPage {
            rows,
            next_ord: page.next_ord,
            row_count: page.row_count,
            meta: page.meta,
        })
    }

    /// The cursor for ordinal `ord` of `result`, issued by this process.
    #[must_use]
    pub fn cursor(&self, result: ResultId, ord: u64) -> Cursor {
        Cursor {
            nonce: self.nonce(),
            result,
            ord,
        }
    }
}

fn is_complete(conn: &Connection) -> Result<bool, ResultStoreError> {
    let state: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'state'", [], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| ResultStoreError::from_sqlite("read the result state", error))?;
    Ok(state.as_deref() == Some("complete"))
}

fn read_rows(
    conn: &Connection,
    columns: usize,
    from_ord: u64,
    accept: &mut impl FnMut(&StoredRow) -> Take,
) -> Result<(Vec<StoredRow>, Option<u64>), ResultStoreError> {
    let sql = format!(
        "SELECT ord, nonfinite, {} FROM rows WHERE ord >= ?1 ORDER BY ord",
        column_list(columns)
    );
    let read = |error| ResultStoreError::from_sqlite("read result rows", error);
    let start = i64::try_from(from_ord).map_err(|_| ResultStoreError::Unknown)?;
    let mut statement = conn.prepare_cached(&sql).map_err(read)?;
    let mut cursor = statement.query([start]).map_err(read)?;
    let mut rows = Vec::new();
    while let Some(raw) = cursor.next().map_err(read)? {
        let row = decode_row(raw, columns)?;
        if !rows.is_empty() && accept(&row) == Take::No {
            return Ok((rows, Some(row.ord)));
        }
        rows.push(row);
    }
    Ok((rows, None))
}

fn decode_row(raw: &rusqlite::Row<'_>, columns: usize) -> Result<StoredRow, ResultStoreError> {
    let read = |error| ResultStoreError::from_sqlite("read a result row", error);
    let ord = u64::try_from(raw.get::<_, i64>(0).map_err(read)?)
        .map_err(|_| ResultStoreError::corrupt("negative row ordinal"))?;
    let marks: Option<String> = raw.get(1).map_err(read)?;
    let mut cells = Vec::with_capacity(columns);
    for index in 0..columns {
        cells.push(match raw.get_ref(index + 2).map_err(read)? {
            ValueRef::Null => Cell::Null,
            ValueRef::Integer(value) => Cell::Int(value),
            ValueRef::Real(value) => Cell::Real(value),
            ValueRef::Text(bytes) => Cell::Text(
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| ResultStoreError::corrupt("stored text is not UTF-8"))?,
            ),
            ValueRef::Blob(bytes) => Cell::Blob(bytes.to_vec()),
        });
    }
    Ok(StoredRow {
        ord,
        cells,
        nonfinite: decode_marks(marks.as_deref())?,
    })
}

fn decode_marks(json: Option<&str>) -> Result<Vec<(usize, NonFinite)>, ResultStoreError> {
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    let named: Vec<(usize, String)> = serde_json::from_str(json)
        .map_err(|error| ResultStoreError::json("decode non-finite marks", error))?;
    named
        .into_iter()
        .map(|(index, name)| {
            NonFinite::parse(&name)
                .map(|mark| (index, mark))
                .ok_or_else(|| ResultStoreError::corrupt("unknown non-finite mark"))
        })
        .collect()
}
