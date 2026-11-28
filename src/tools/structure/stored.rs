//! Stored row layout, tool-level extras, byte-budget row cut, and response
//! assembly for `structure`.

use super::{StructureMatch, StructureResponse};
use crate::error_codes::store_error;
use crate::store::{Cell, Materialize, ResultStoreError, StoredRow};
use crate::tools::page::Page;
use crate::tools::paginate::{CutStep, ServedRows, byte_cut_step};
use crate::tools::untested_language_notices;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Json;
use serde::{Deserialize, Serialize};

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

/// One stored row: the match.
#[derive(Debug)]
pub(in crate::tools) struct StoredMatch {
    pub(in crate::tools) found: StructureMatch,
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
        })
    }
}

/// Tool-level facts stored with a result and returned unchanged on every page.
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

/// Single-decode byte cut for one stored [`StructureMatch`] (per-page row cap
/// `max_structure_matches` is enforced upstream by `PageSpec::max_rows`).
pub(in crate::tools) fn cut_match_row(
    row: &StoredRow,
    taken: usize,
    bytes_so_far: usize,
    page_bytes: usize,
) -> Result<CutStep<StructureMatch>, ErrorData> {
    let stored = StoredMatch::from_values(&row.cells).map_err(|error| store_error(&error))?;
    let bytes = match_bytes(&stored.found)?;
    Ok(byte_cut_step(
        stored.found,
        bytes,
        taken,
        bytes_so_far,
        page_bytes,
    ))
}

/// Logs the call's facts and assembles the response from a served page.
pub(super) fn structure_response(
    served: ServedRows<StructureMatch, StructureExtra>,
) -> Json<StructureResponse> {
    let matches = served.rows;
    let extra = served.meta;
    tracing::info!(
        match_count = matches.len(),
        limited_by = ?served.page.limited_by,
        files_not_scanned = extra.files_not_scanned,
        unparsed = extra.unparsed_files.len(),
        over_parse_limit = extra.files_over_parse_limit,
        parse_timed_out = extra.parse_timed_out_file.is_some(),
        page_bytes = served.page.page_bytes,
        "structure executed"
    );
    let page = Page {
        total: None,
        ..served.page
    };
    Json(StructureResponse {
        page,
        notices: untested_language_notices([(extra.language.as_str(), None)]),
        language: extra.language,
        matches,
        files_not_scanned: extra.files_not_scanned,
        bytes_not_scanned: extra.bytes_not_scanned,
        next_start_after_file: extra.next_start_after_file,
        unparsed_files: extra.unparsed_files,
        files_over_parse_limit: extra.files_over_parse_limit,
        parse_timed_out_file: extra.parse_timed_out_file,
    })
}
