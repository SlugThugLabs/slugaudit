// slugaudit-line-exception: approved-by=slugthug; reason=the stored finding_read result: its row layout, the streaming session read that fills it and the byte cut that reads it back share one row shape and one serialized-size measure
//! The stored shape of a `finding_read` result: the row layout kept in the
//! result store, the streaming read of the session's findings that fills it,
//! the byte cut that sizes a page, and the clip of an oversized first row.

use super::FindingReadEntry;
use crate::error_codes::store_error;
use crate::store::{Cell, Materialize, ResultStoreError, StoredRow, Take};
use crate::tools::context::internal_error;
use crate::tools::paginate::RowSink;
use rmcp::ErrorData;
use serde::{Deserialize, Serialize};

/// A finding as stored in the result.
pub(super) struct StoredFinding {
    pub(super) entry: FindingReadEntry,
}

/// What is stored with the result and returned on every page: whether the
/// budget of the first page was the ceiling, so a continuation reports the
/// same `limited_by` without reading any limits.
#[derive(Serialize, Deserialize)]
pub(super) struct FindingExtra {
    pub(super) at_ceiling: bool,
}

impl Materialize for StoredFinding {
    fn columns() -> &'static [&'static str] {
        &[
            "id",
            "path",
            "line_start",
            "line_end",
            "severity",
            "category",
            "title",
            "description",
            "status",
            "source_hash",
        ]
    }

    fn to_values(&self) -> Vec<Cell> {
        let entry = &self.entry;
        vec![
            Cell::Int(entry.id),
            Cell::Text(entry.path.clone()),
            Cell::Int(i64::from(entry.line_start)),
            Cell::Int(i64::from(entry.line_end)),
            Cell::Text(entry.severity.clone()),
            Cell::Text(entry.category.clone()),
            Cell::Text(entry.title.clone()),
            Cell::Text(entry.description.clone()),
            Cell::Text(entry.status.clone()),
            Cell::Text(entry.source_hash.clone()),
        ]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        const LAYOUT: ResultStoreError = ResultStoreError::Invalid {
            reason: "finding_read row layout",
        };
        let line = |cell: &Cell| match cell {
            Cell::Int(value) => u32::try_from(*value).map_err(|_| LAYOUT),
            _ => Err(LAYOUT),
        };
        let text = |cell: &Cell| match cell {
            Cell::Text(value) => Ok(value.clone()),
            _ => Err(LAYOUT),
        };
        let [
            Cell::Int(id),
            path,
            line_start,
            line_end,
            severity,
            category,
            title,
            description,
            status,
            source_hash,
        ] = values
        else {
            return Err(LAYOUT);
        };
        Ok(Self {
            entry: FindingReadEntry {
                id: *id,
                path: text(path)?,
                line_start: line(line_start)?,
                line_end: line(line_end)?,
                severity: text(severity)?,
                category: text(category)?,
                title: text(title)?,
                description: text(description)?,
                detail_clipped: false,
                status: text(status)?,
                source_hash: text(source_hash)?,
            },
        })
    }
}

/// Streams the session's findings into the result, inside the verified read
/// transaction.
pub(super) fn produce(
    tx: &rusqlite::Transaction<'_>,
    scope: SessionScope<'_>,
    rows: &mut RowSink<'_, '_>,
) -> Result<(), ErrorData> {
    each_session_finding(tx, scope, |entry| rows.push(&StoredFinding { entry }))
}

/// The byte cut of one page: rows are taken while their serialized sizes fit
/// the budget. The first row of a page is offered too and always taken
/// (clipped afterwards), so paging always makes progress.
pub(super) struct PageCut {
    budget: usize,
    used: Option<usize>,
    failure: Option<ErrorData>,
}

impl PageCut {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            budget,
            used: None,
            failure: None,
        }
    }

    pub(super) fn accept(&mut self, row: &StoredRow) -> Take {
        match self.measure(row) {
            Ok(take) => take,
            Err(error) => {
                self.failure.get_or_insert(error);
                Take::No
            }
        }
    }

    fn measure(&mut self, row: &StoredRow) -> Result<Take, ErrorData> {
        let stored = StoredFinding::from_values(&row.cells).map_err(|e| store_error(&e))?;
        let size = serialized_len(&stored.entry)?;
        let Some(used) = self.used else {
            // The first row of a page counts whatever its size.
            self.used = Some(size);
            return Ok(Take::Yes);
        };
        if used.saturating_add(size) > self.budget {
            return Ok(Take::No);
        }
        self.used = Some(used.saturating_add(size));
        Ok(Take::Yes)
    }

    /// Surfaces the failure the cut hit, if any.
    pub(super) fn finish(self) -> Result<(), ErrorData> {
        self.failure.map_or(Ok(()), Err)
    }
}

/// Which findings a read selects: the calling session's, optionally for one
/// file.
pub(super) type SessionScope<'a> = (&'a str, Option<&'a str>);

/// Calls `each` with every finding the scope selects, in id order.
pub(super) fn each_session_finding(
    tx: &rusqlite::Transaction<'_>,
    (current_session, file_filter): SessionScope<'_>,
    mut each: impl FnMut(FindingReadEntry) -> Result<(), ErrorData>,
) -> Result<(), ErrorData> {
    let (filter, query_params) = session_filter(current_session, file_filter);
    let sql = format!(
        "SELECT id, path, line_start, line_end, severity, category, \
             title, description, status, source_hash \
         FROM findings WHERE {filter} ORDER BY id"
    );
    let mut statement = tx
        .prepare(&sql)
        .map_err(|error| internal_error("preparing the session findings read", error))?;
    let rows = statement
        .query_map(
            rusqlite::params_from_iter(query_params.iter()),
            finding_entry,
        )
        .map_err(|error| internal_error("executing the session findings read", error))?;
    for row in rows {
        each(row.map_err(|error| internal_error("reading a session finding", error))?)?;
    }
    Ok(())
}

/// The WHERE clause and bound parameters that select one session's
/// findings.
fn session_filter(
    current_session: &str,
    file_filter: Option<&str>,
) -> (&'static str, Vec<rusqlite::types::Value>) {
    let mut query_params = vec![rusqlite::types::Value::Text(current_session.to_owned())];
    let Some(file) = file_filter else {
        return ("session_id = ?1", query_params);
    };
    query_params.push(rusqlite::types::Value::Text(file.to_owned()));
    ("session_id = ?1 AND path = ?2", query_params)
}

/// Maps one findings row to its response entry.
fn finding_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<FindingReadEntry> {
    Ok(FindingReadEntry {
        id: row.get(0)?,
        path: row.get(1)?,
        line_start: row.get(2)?,
        line_end: row.get(3)?,
        severity: row.get(4)?,
        category: row.get(5)?,
        title: row.get(6)?,
        description: row.get(7)?,
        detail_clipped: false,
        status: row.get(8)?,
        source_hash: row.get(9)?,
    })
}

/// Clips `description` to the longest character prefix whose serialized row
/// fits `budget`, setting `detail_clipped` only when characters were
/// removed. Restores the whole text when not even an empty description
/// fits, so an oversized row is returned whole rather than silently
/// emptied (its `page.bytes_used` then exceeds `page.page_bytes`, which the
/// response reports as-is).
pub(super) fn clip_description(
    entry: &mut FindingReadEntry,
    budget: usize,
) -> Result<(), ErrorData> {
    let chars: Vec<char> = entry.description.chars().collect();
    let original = entry.description.clone();
    // The flag is part of the serialized row, so the search must measure the
    // row as it will actually be returned (flag set), never one byte short.
    entry.detail_clipped = true;
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        entry.description = chars[..mid].iter().collect();
        if serialized_len(entry)? <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    entry.description = chars[..low].iter().collect();
    if low == chars.len() || (low == 0 && serialized_len(entry)? > budget) {
        entry.description = original;
        entry.detail_clipped = false;
    }
    Ok(())
}

pub(super) fn serialized_len(entry: &FindingReadEntry) -> Result<usize, ErrorData> {
    crate::util::serialized_len(entry)
        .map_err(|error| internal_error("measuring a session finding's response size", error))
}
