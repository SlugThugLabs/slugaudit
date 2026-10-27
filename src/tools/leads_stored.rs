//! What a `leads` result stores: the row layout (the response row plus what a
//! page cut needs), the extras returned unchanged on every page, and the
//! [`Feed`] a producer pushes rows through.

use crate::store::{Cell, Materialize, ResultStoreError};
use crate::tools::context::{RowSink, internal_error};
use crate::tools::leads::{CompactLeadRow, LeadRow, MechanicCount};
use crate::tools::mechanic::MechanicSource;
use crate::util::serialized_len;
use rmcp::ErrorData;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One stored row of either mode: the response row's fields (`detail` and
/// `detail_clipped` stay empty in compact mode; the extras say which mode the
/// result is) plus what a page cut needs without reading earlier rows.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StoredLead {
    pub(super) mechanic: String,
    pub(super) file: String,
    pub(super) line: i64,
    pub(super) detail: String,
    pub(super) detail_clipped: bool,
    /// Distinct files in this row and every row before it.
    pub(super) files_seen: i64,
    /// Serialized JSON bytes of this row as the response carries it.
    pub(super) bytes: i64,
    /// The same for the row before this one (0 for the first): the page cut
    /// is never offered the first row of a page, whose size it needs.
    pub(super) prev_bytes: i64,
}

const COLUMNS: &[&str] = &[
    "mechanic",
    "file",
    "line",
    "detail",
    "detail_clipped",
    "files_seen",
    "bytes",
    "prev_bytes",
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
            Cell::Int(self.bytes),
            Cell::Int(self.prev_bytes),
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
                Cell::Int(bytes),
                Cell::Int(prev_bytes),
            ] => Ok(Self {
                mechanic: mechanic.clone(),
                file: file.clone(),
                line: *line,
                detail: detail.clone(),
                detail_clipped: *clipped != 0,
                files_seen: *files_seen,
                bytes: *bytes,
                prev_bytes: *prev_bytes,
            }),
            _ => Err(ResultStoreError::Invalid {
                reason: "leads row layout",
            }),
        }
    }
}

/// `MechanicSource`'s wire form, for the stored counts.
#[derive(Serialize, Deserialize)]
#[serde(remote = "MechanicSource", rename_all = "snake_case")]
enum SourceForm {
    Session,
    ProjectConfig,
    Builtin,
}

/// A [`MechanicCount`] as stored with the result.
#[derive(Serialize, Deserialize)]
pub(super) struct StoredCount {
    pub(super) mechanic: String,
    #[serde(with = "SourceForm")]
    source: MechanicSource,
    note: Option<String>,
    count: i64,
    files: i64,
    covers: Vec<String>,
}

/// What is stored with the result and returned unchanged on every page.
#[derive(Serialize, Deserialize)]
pub(super) struct Extras {
    pub(super) counts: Vec<StoredCount>,
    pub(super) mechanics_invalid: usize,
    pub(super) files_not_parsed: i64,
    /// Distinct files across every stored row; `0` for a counts-only call.
    pub(super) files_total: i64,
    /// Whether the rows are compact triples.
    pub(super) compact: bool,
}

/// Stores rows, tracking the distinct files and the row sizes the page cut
/// needs.
pub(in crate::tools) struct Feed<'a, 'w, 's> {
    sink: &'a mut RowSink<'w, 's>,
    seen: HashSet<String>,
    pub(super) prev_bytes: i64,
}

impl<'a, 'w, 's> Feed<'a, 'w, 's> {
    pub(super) fn new(sink: &'a mut RowSink<'w, 's>) -> Self {
        Self {
            sink,
            seen: HashSet::new(),
            prev_bytes: 0,
        }
    }

    /// Distinct files across the rows stored so far.
    pub(super) fn files(&self) -> usize {
        self.seen.len()
    }

    /// Stores one full-mode row.
    pub(in crate::tools) fn full(&mut self, row: LeadRow) -> Result<(), ErrorData> {
        let bytes = measure(&row)?;
        self.store(StoredLead {
            mechanic: String::new(),
            file: row.file,
            line: row.line,
            detail: row.detail,
            detail_clipped: row.detail_clipped,
            files_seen: 0,
            bytes,
            prev_bytes: 0,
        })
    }

    /// Stores one compact row.
    pub(in crate::tools) fn compact(&mut self, row: CompactLeadRow) -> Result<(), ErrorData> {
        let bytes = measure(&row)?;
        self.store(StoredLead {
            mechanic: row.mechanic,
            file: row.file,
            line: row.line,
            detail: String::new(),
            detail_clipped: false,
            files_seen: 0,
            bytes,
            prev_bytes: 0,
        })
    }

    fn store(&mut self, mut row: StoredLead) -> Result<(), ErrorData> {
        self.seen.insert(row.file.clone());
        row.files_seen = to_i64(self.seen.len())?;
        row.prev_bytes = self.prev_bytes;
        self.prev_bytes = row.bytes;
        self.sink.push(&row)
    }
}

fn measure(row: &impl Serialize) -> Result<i64, ErrorData> {
    let bytes =
        serialized_len(row).map_err(|error| internal_error("measuring a leads row", error))?;
    to_i64(bytes)
}

pub(super) fn to_i64(value: usize) -> Result<i64, ErrorData> {
    i64::try_from(value).map_err(|error| internal_error("counting leads rows", error))
}

pub(super) fn count_of(stored: StoredCount) -> MechanicCount {
    MechanicCount {
        mechanic: stored.mechanic,
        source: stored.source,
        note: stored.note,
        count: stored.count,
        files: stored.files,
        covers: stored.covers,
    }
}

pub(super) fn stored_of(count: MechanicCount) -> StoredCount {
    StoredCount {
        mechanic: count.mechanic,
        source: count.source,
        note: count.note,
        count: count.count,
        files: count.files,
        covers: count.covers,
    }
}
