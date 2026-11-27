//! Stored row layout, response byte sizing, result extras, and row [`Feed`]
//! for `leads` pagination.

use crate::store::{Cell, Materialize, ResultStoreError};
use crate::tools::context::internal_error;
use crate::tools::leads::{CompactLeadRow, LeadRow, MechanicCount};
use crate::tools::paginate::RowSink;
use crate::util::serialized_len;
use rmcp::ErrorData;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One stored row of either mode: the response row's fields (`detail` and
/// `detail_clipped` stay empty in compact mode; the extras say which mode the
/// result is) plus what a page needs without reading earlier rows.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StoredLead {
    pub(super) mechanic: String,
    pub(super) file: String,
    pub(super) line: i64,
    pub(super) detail: String,
    pub(super) detail_clipped: bool,
    /// Distinct files in this row and every row before it.
    pub(super) files_seen: i64,
}

impl StoredLead {
    /// Whether this is a compact row: a compact row names its mechanic, a
    /// full row (the mechanic is selected by the request) leaves it empty.
    pub(super) fn is_compact(&self) -> bool {
        !self.mechanic.is_empty()
    }

    /// The response's compact form of this row.
    pub(super) fn into_compact(self) -> CompactLeadRow {
        CompactLeadRow {
            mechanic: self.mechanic,
            file: self.file,
            line: self.line,
        }
    }

    /// The response's full form of this row.
    pub(super) fn into_full(self) -> LeadRow {
        LeadRow {
            file: self.file,
            line: self.line,
            detail: self.detail,
            detail_clipped: self.detail_clipped,
        }
    }

    /// Serialized JSON bytes of this row as the response carries it, measured
    /// on a borrowed view with the same fields as [`CompactLeadRow`] or
    /// [`LeadRow`], without copying the row.
    pub(in crate::tools) fn response_bytes(&self) -> Result<usize, ErrorData> {
        #[derive(Serialize)]
        struct CompactView<'a> {
            mechanic: &'a str,
            file: &'a str,
            line: i64,
        }
        #[derive(Serialize)]
        struct FullView<'a> {
            file: &'a str,
            line: i64,
            detail: &'a str,
            detail_clipped: bool,
        }
        if self.is_compact() {
            serialized_len(&CompactView {
                mechanic: &self.mechanic,
                file: &self.file,
                line: self.line,
            })
        } else {
            serialized_len(&FullView {
                file: &self.file,
                line: self.line,
                detail: &self.detail,
                detail_clipped: self.detail_clipped,
            })
        }
        .map_err(|error| internal_error("measuring a leads row", error))
    }
}

const COLUMNS: &[&str] = &[
    "mechanic",
    "file",
    "line",
    "detail",
    "detail_clipped",
    "files_seen",
];

impl Materialize for StoredLead {
    fn columns() -> &'static [&'static str] {
        COLUMNS
    }

    fn to_values(&self) -> Vec<Cell> {
        vec![
            Cell::Text(self.mechanic.clone()),
            Cell::Text(self.file.clone()),
            Cell::Int(self.line),
            Cell::Text(self.detail.clone()),
            Cell::Int(i64::from(self.detail_clipped)),
            Cell::Int(self.files_seen),
        ]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        match values {
            [
                Cell::Text(mechanic),
                Cell::Text(file),
                Cell::Int(line),
                Cell::Text(detail),
                Cell::Int(clipped),
                Cell::Int(files_seen),
            ] => Ok(Self {
                mechanic: mechanic.clone(),
                file: file.clone(),
                line: *line,
                detail: detail.clone(),
                detail_clipped: *clipped != 0,
                files_seen: *files_seen,
            }),
            _ => Err(ResultStoreError::Invalid {
                reason: "leads row layout",
            }),
        }
    }
}

/// What is stored with the result and returned unchanged on every page.
#[derive(Serialize, Deserialize)]
pub(super) struct Extras {
    pub(super) counts: Vec<MechanicCount>,
    pub(super) notices: Vec<String>,
    pub(super) files_not_parsed: i64,
    /// Distinct files across every stored row; `0` for a counts-only call.
    pub(super) files_total: i64,
    /// Whether the rows are compact triples.
    pub(super) compact: bool,
    /// The row `limit` page one was asked for; a continuation without a
    /// `limit` of its own keeps it.
    pub(super) limit: Option<usize>,
}

/// Stores rows, tracking the distinct files.
pub(in crate::tools) struct Feed<'a, 'w, 's> {
    sink: &'a mut RowSink<'w, 's>,
    seen: HashSet<String>,
}

impl<'a, 'w, 's> Feed<'a, 'w, 's> {
    pub(super) fn new(sink: &'a mut RowSink<'w, 's>) -> Self {
        Self {
            sink,
            seen: HashSet::new(),
        }
    }

    /// Distinct files across the rows stored so far.
    pub(super) fn files(&self) -> usize {
        self.seen.len()
    }

    /// Stores one full-mode row.
    pub(in crate::tools) fn full(&mut self, row: LeadRow) -> Result<(), ErrorData> {
        self.store(StoredLead {
            mechanic: String::new(),
            file: row.file,
            line: row.line,
            detail: row.detail,
            detail_clipped: row.detail_clipped,
            files_seen: 0,
        })
    }

    /// Stores one compact row.
    pub(in crate::tools) fn compact(&mut self, row: CompactLeadRow) -> Result<(), ErrorData> {
        self.store(StoredLead {
            mechanic: row.mechanic,
            file: row.file,
            line: row.line,
            detail: String::new(),
            detail_clipped: false,
            files_seen: 0,
        })
    }

    fn store(&mut self, mut row: StoredLead) -> Result<(), ErrorData> {
        self.seen.insert(row.file.clone());
        row.files_seen = to_i64(self.seen.len())?;
        self.sink.push(&row)
    }
}

pub(super) fn to_i64(value: usize) -> Result<i64, ErrorData> {
    i64::try_from(value).map_err(|error| internal_error("counting leads rows", error))
}
