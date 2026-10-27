// slugaudit-line-exception: approved-by=slugthug; reason=one stored layout: the row type, its decode and the page cut that reads it must change together
//! The stored layout of a `structure` result: one row per match with its
//! serialized size, the tool-level extras returned on every page, and the
//! page cut applied while a stored page is read.

use crate::error_codes::store_error;
use crate::store::{Cell, Materialize, ResultStoreError, StoredRow, Take};
use crate::tools::page::PageLimit;
use crate::tools::page_size::PageSize;
use crate::tools::structure::StructureMatch;
use rmcp::ErrorData;
use serde::{Deserialize, Serialize};

/// Measures one match as it serializes into a page.
fn match_bytes(found: &StructureMatch) -> Result<usize, ErrorData> {
    serde_json::to_vec(found)
        .map(|bytes| bytes.len())
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

const MATCH_LAYOUT: &str = "structure match row layout";

fn invalid_layout() -> ResultStoreError {
    ResultStoreError::Invalid {
        reason: MATCH_LAYOUT,
    }
}

/// A byte offset travels as the same 64 bits in an SQLite integer.
fn int_of(value: u64) -> Cell {
    Cell::Int(i64::from_ne_bytes(value.to_ne_bytes()))
}

fn u64_of(cell: &Cell) -> Result<u64, ResultStoreError> {
    match cell {
        Cell::Int(value) => Ok(u64::from_ne_bytes(value.to_ne_bytes())),
        _ => Err(invalid_layout()),
    }
}

fn u32_of(cell: &Cell) -> Result<u32, ResultStoreError> {
    match cell {
        Cell::Int(value) => u32::try_from(*value).map_err(|_| invalid_layout()),
        _ => Err(invalid_layout()),
    }
}

fn bool_of(cell: &Cell) -> Result<bool, ResultStoreError> {
    match cell {
        Cell::Int(0) => Ok(false),
        Cell::Int(1) => Ok(true),
        _ => Err(invalid_layout()),
    }
}

fn text_of(cell: &Cell) -> Result<String, ResultStoreError> {
    match cell {
        Cell::Text(text) => Ok(text.clone()),
        _ => Err(invalid_layout()),
    }
}

fn usize_of(cell: &Cell) -> Result<usize, ResultStoreError> {
    usize::try_from(u64_of(cell)?).map_err(|_| invalid_layout())
}

/// One stored row: the match plus its serialized size and the size of the
/// row before it. The store never offers a page's first row to the page cut,
/// so the second row of every page carries the first row's size and the byte
/// budget stays exact from the first match on.
#[derive(Debug)]
pub(in crate::tools) struct StoredMatch {
    pub(in crate::tools) found: StructureMatch,
    /// Serialized JSON bytes of `found`.
    pub(in crate::tools) bytes: usize,
    /// The same for the previous row; 0 for the first.
    pub(in crate::tools) prev_bytes: usize,
}

impl StoredMatch {
    /// Measures `found` as it serializes into a page.
    pub(in crate::tools) fn measured(
        found: StructureMatch,
        prev_bytes: usize,
    ) -> Result<Self, ErrorData> {
        let bytes = match_bytes(&found)?;
        Ok(Self {
            found,
            bytes,
            prev_bytes,
        })
    }
}

impl Materialize for StoredMatch {
    fn columns() -> &'static [&'static str] {
        &[
            "file",
            "capture_name",
            "node_kind",
            "start_byte",
            "end_byte",
            "start_line",
            "start_column",
            "end_line",
            "end_column",
            "text",
            "text_truncated",
            "extraction_failed",
            "json_bytes",
            "prev_json_bytes",
        ]
    }

    fn to_values(&self) -> Vec<Cell> {
        let found = &self.found;
        vec![
            Cell::Text(found.file.clone()),
            Cell::Text(found.capture_name.clone()),
            Cell::Text(found.node_kind.clone()),
            int_of(found.start_byte),
            int_of(found.end_byte),
            Cell::Int(i64::from(found.start_line)),
            Cell::Int(i64::from(found.start_column)),
            Cell::Int(i64::from(found.end_line)),
            Cell::Int(i64::from(found.end_column)),
            Cell::Text(found.text.clone()),
            Cell::Int(i64::from(found.text_truncated)),
            Cell::Int(i64::from(found.extraction_failed)),
            int_of(self.bytes as u64),
            int_of(self.prev_bytes as u64),
        ]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        let [
            file,
            capture_name,
            node_kind,
            start_byte,
            end_byte,
            start_line,
            start_column,
            end_line,
            end_column,
            text,
            text_truncated,
            extraction_failed,
            bytes,
            prev_bytes,
        ] = values
        else {
            return Err(invalid_layout());
        };
        Ok(Self {
            found: StructureMatch {
                file: text_of(file)?,
                capture_name: text_of(capture_name)?,
                node_kind: text_of(node_kind)?,
                start_byte: u64_of(start_byte)?,
                end_byte: u64_of(end_byte)?,
                start_line: u32_of(start_line)?,
                start_column: u32_of(start_column)?,
                end_line: u32_of(end_line)?,
                end_column: u32_of(end_column)?,
                text: text_of(text)?,
                text_truncated: bool_of(text_truncated)?,
                extraction_failed: bool_of(extraction_failed)?,
            },
            bytes: usize_of(bytes)?,
            prev_bytes: usize_of(prev_bytes)?,
        })
    }
}

/// The tool-level facts stored with a result and returned unchanged on every
/// page.
#[derive(Debug, Serialize, Deserialize)]
pub(in crate::tools) struct StructureExtra {
    pub(in crate::tools) language: String,
    pub(in crate::tools) files_not_scanned: usize,
    pub(in crate::tools) bytes_not_scanned: u64,
    pub(in crate::tools) next_start_after_file: Option<String>,
    pub(in crate::tools) unparsed_files: Vec<String>,
    pub(in crate::tools) files_over_parse_limit: usize,
    pub(in crate::tools) parse_timed_out_file: Option<String>,
}

/// Decides where one stored page ends: at `max_matches` rows or when the next
/// match would push the serialized bytes past the page budget. The store
/// always keeps a page's first row, so every page advances.
pub(in crate::tools) struct PageCut {
    page_bytes: usize,
    byte_cut: PageLimit,
    max_matches: usize,
    taken: usize,
    bytes_used: usize,
    cut: PageLimit,
    failure: Option<ErrorData>,
}

impl PageCut {
    pub(in crate::tools) fn new(page: PageSize, max_matches: usize) -> Self {
        Self {
            page_bytes: page.bytes,
            byte_cut: page.byte_cut(),
            max_matches,
            taken: 0,
            bytes_used: 0,
            cut: PageLimit::None,
            failure: None,
        }
    }

    /// The `accept` closure of `paginate`. The store never offers it a
    /// page's first row, so the first call (the page's second row) starts
    /// from the first row's size, which that row carries.
    pub(in crate::tools) fn accept(&mut self, row: &StoredRow) -> Take {
        if self.failure.is_some() {
            return Take::No;
        }
        let stored = match StoredMatch::from_values(&row.cells) {
            Ok(stored) => stored,
            Err(error) => {
                self.failure = Some(store_error(&error));
                return Take::No;
            }
        };
        if self.taken == 0 {
            self.taken = 1;
            self.bytes_used = stored.prev_bytes;
        }
        if self.taken >= self.max_matches {
            self.cut = PageLimit::MatchCap;
            return Take::No;
        }
        if self.bytes_used.saturating_add(stored.bytes) > self.page_bytes {
            self.cut = self.byte_cut;
            return Take::No;
        }
        self.taken += 1;
        self.bytes_used = self.bytes_used.saturating_add(stored.bytes);
        Take::Yes
    }

    /// What ended the page, `none` when the rows simply ran out.
    pub(in crate::tools) fn limit(&self) -> PageLimit {
        self.cut
    }

    /// The failure `accept` met, if any.
    pub(in crate::tools) fn check(&mut self) -> Result<(), ErrorData> {
        self.failure.take().map_or(Ok(()), Err)
    }
}
