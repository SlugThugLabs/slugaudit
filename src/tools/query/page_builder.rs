//! Byte-budgeted [`PageBuilder`] and raw-cell size lower bound for `query` pages.

use super::clip::{ClippedCell, clip_row};
use super::value::{
    NonFiniteCell, build_row_object, ensure_unique_column_names, into_json_value, non_finite_cells,
    sql_value_to_json, values_of,
};
use crate::store::StoredRow;
use crate::tools::context::internal_error;
use crate::tools::paginate::CutStep;
use crate::util;
use rmcp::ErrorData;
use rusqlite::types::Value as SqlValue;

/// One page of a result plus sideband cell metadata.
pub(super) struct Collected {
    pub(super) rows: Vec<serde_json::Value>,
    pub(super) clipped: Vec<ClippedCell>,
    pub(super) non_finite: Vec<NonFiniteCell>,
}

/// Cuts a stored result into one byte-budgeted page of JSON rows.
pub(super) struct PageBuilder {
    names: Vec<String>,
    page_bytes: usize,
    clipped: Vec<ClippedCell>,
    non_finite: Vec<NonFiniteCell>,
}

impl PageBuilder {
    pub(super) fn new() -> Self {
        Self {
            names: Vec::new(),
            page_bytes: 0,
            clipped: Vec::new(),
            non_finite: Vec::new(),
        }
    }

    pub(super) fn configure(
        &mut self,
        names: Vec<String>,
        page_bytes: usize,
    ) -> Result<(), ErrorData> {
        ensure_unique_column_names(&names)?;
        self.names = names;
        self.page_bytes = page_bytes;
        self.clipped.clear();
        self.non_finite.clear();
        Ok(())
    }

    fn take_first(
        &mut self,
        values: &[SqlValue],
        ord: u64,
    ) -> Result<CutStep<serde_json::Value>, ErrorData> {
        let row_index = row_index_of(ord)?;
        let (row, bytes, cells) = if row_serialized_lower_bound(values) > self.page_bytes {
            self.clip(values, row_index)?
        } else {
            let row = build_row_object(&self.names, values.iter().map(sql_value_to_json));
            let bytes = row_bytes(&row)?;
            if bytes > self.page_bytes {
                self.clip(values, row_index)?
            } else {
                (row, bytes, Vec::new())
            }
        };
        self.clipped = cells;
        self.non_finite
            .extend(non_finite_cells(&self.names, values, row_index));
        Ok(CutStep::Keep { row, bytes })
    }

    fn clip(
        &self,
        values: &[SqlValue],
        row_index: usize,
    ) -> Result<(serde_json::Value, usize, Vec<ClippedCell>), ErrorData> {
        let cut = clip_row(&self.names, values, self.page_bytes, row_index)?;
        let bytes = row_bytes(&cut.row)?;
        Ok((cut.row, bytes, cut.cells))
    }

    pub(super) fn cut_row(
        &mut self,
        row: &StoredRow,
        taken: usize,
        bytes_so_far: usize,
    ) -> Result<CutStep<serde_json::Value>, ErrorData> {
        let values = values_of(row);
        if taken == 0 {
            return self.take_first(&values, row.ord);
        }
        let row_index = row_index_of(row.ord)?;
        if row_serialized_lower_bound(&values) > self.page_bytes {
            return Ok(CutStep::Stop { limit: None });
        }
        let non_finite = non_finite_cells(&self.names, &values, row_index);
        let object = build_row_object(&self.names, values.into_iter().map(into_json_value));
        let bytes = row_bytes(&object)?;
        if bytes_so_far
            .checked_add(bytes)
            .is_none_or(|total| total > self.page_bytes)
        {
            return Ok(CutStep::Stop { limit: None });
        }
        self.non_finite.extend(non_finite);
        Ok(CutStep::Keep { row: object, bytes })
    }

    pub(super) fn into_collected(self, rows: Vec<serde_json::Value>) -> Collected {
        Collected {
            rows,
            clipped: self.clipped,
            non_finite: self.non_finite,
        }
    }
}

fn row_bytes(value: &serde_json::Value) -> Result<usize, ErrorData> {
    util::serialized_len(value).map_err(|error| internal_error("measuring query row size", error))
}

fn row_serialized_lower_bound(values: &[SqlValue]) -> usize {
    values.iter().fold(0usize, |total, value| {
        let contribution = match value {
            SqlValue::Text(text) => text.len(),
            // KEEP saturating: a lower bound clamped at usize::MAX still exceeds any finite page budget
            SqlValue::Blob(bytes) => bytes.len().saturating_mul(2),
            SqlValue::Null | SqlValue::Integer(_) | SqlValue::Real(_) => 0,
        };
        // KEEP saturating: a lower bound clamped at usize::MAX still exceeds any finite page budget
        total.saturating_add(contribution)
    })
}

fn row_index_of(ord: u64) -> Result<usize, ErrorData> {
    usize::try_from(ord).map_err(|error| internal_error("reading a result row position", error))
}
