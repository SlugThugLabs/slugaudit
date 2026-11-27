//! Producer row sink for [`super::paginate`]: buffers page-one rows in
//! memory first and spills to [`super::paginate_store::StoreSession`] only when
//! a result crosses the in-memory cap or spans multiple pages.

use super::paginate_store::{StoreSession, map_store};
use crate::error_codes::store_error;
use crate::store::{Cell, Materialize, ResultStoreError, ResultWriter};
use crate::tools::context::internal_error;
use rmcp::ErrorData;

const MAX_RESULT_COLUMNS: usize = 1_000;
const MIN_BUFFER_BYTES: usize = 4 * 1024;
const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const MAX_BUFFER_ROWS: usize = 65_536;
const MIN_ROW_OVERHEAD_BYTES: usize = 8;

#[derive(Debug, thiserror::Error)]
enum SinkError {
    #[error("the result columns were already declared")]
    ColumnsAlreadyDeclared,
    #[error("a row was pushed before the result columns were declared")]
    ColumnsNotDeclared,
}

pub(super) enum SinkMode<'s> {
    Buffered {
        rows: Vec<Vec<Cell>>,
        est_bytes: usize,
    },
    Spilled(Box<ResultWriter<'s>>),
}

pub(super) struct SinkTarget<'s> {
    store: &'s StoreSession<'s>,
    columns: Option<Vec<String>>,
    buffer_byte_cap: usize,
    mode: SinkMode<'s>,
}

pub(super) struct FinishedSink<'s> {
    pub(super) columns: Vec<String>,
    pub(super) mode: SinkMode<'s>,
}

impl<'s> SinkTarget<'s> {
    pub(super) fn new(
        store: &'s StoreSession<'s>,
        spec_columns: &[String],
        page_bytes: usize,
    ) -> Result<Self, ErrorData> {
        let columns = if spec_columns.is_empty() {
            None
        } else {
            validate_columns(spec_columns)?;
            Some(spec_columns.to_vec())
        };
        Ok(Self {
            store,
            columns,
            buffer_byte_cap: page_bytes.clamp(MIN_BUFFER_BYTES, MAX_BUFFER_BYTES),
            mode: SinkMode::Buffered {
                rows: Vec::new(),
                est_bytes: 0,
            },
        })
    }

    pub(super) fn finish(self) -> Result<FinishedSink<'s>, ErrorData> {
        let Some(columns) = self.columns else {
            return Err(internal_error(
                "storing the result",
                SinkError::ColumnsNotDeclared,
            ));
        };
        Ok(FinishedSink {
            columns,
            mode: self.mode,
        })
    }
}

fn validate_columns(columns: &[String]) -> Result<(), ErrorData> {
    if columns.is_empty() || columns.len() > MAX_RESULT_COLUMNS {
        return Err(store_error(&ResultStoreError::Invalid {
            reason: "a result needs between 1 and 1000 columns",
        }));
    }
    Ok(())
}

fn checked_row_bytes(cells: &[Cell]) -> Result<usize, ErrorData> {
    cells
        .iter()
        .try_fold(MIN_ROW_OVERHEAD_BYTES, |acc, cell| {
            let len = match cell {
                Cell::Null | Cell::Int(_) | Cell::Real(_) => 8,
                Cell::Text(text) => text.len(),
                Cell::Blob(bytes) => bytes.len(),
            };
            acc.checked_add(len)
        })
        .ok_or_else(|| {
            store_error(&ResultStoreError::Invalid {
                reason: "a row is too large to measure",
            })
        })
}

/// The producer's handle for streaming rows into the page buffer or result store.
pub struct RowSink<'w, 's> {
    pub(super) target: &'w mut SinkTarget<'s>,
}

impl RowSink<'_, '_> {
    /// Declares the result's columns once, before the first push.
    ///
    /// # Errors
    ///
    /// An internal error when columns were already declared, or a mapped store error.
    pub fn set_columns(&mut self, columns: Vec<String>) -> Result<(), ErrorData> {
        if self.target.columns.is_some() {
            return Err(internal_error(
                "declaring the result columns",
                SinkError::ColumnsAlreadyDeclared,
            ));
        }
        validate_columns(&columns)?;
        self.target.columns = Some(columns);
        Ok(())
    }

    /// Stores one typed row.
    ///
    /// # Errors
    ///
    /// A store failure mapped to its wire code.
    pub fn push<R: Materialize>(&mut self, row: &R) -> Result<(), ErrorData> {
        self.push_cells(&row.to_values())
    }

    /// Stores one raw row (`query`).
    ///
    /// # Errors
    ///
    /// A store failure mapped to its wire code.
    pub fn push_cells(&mut self, cells: &[Cell]) -> Result<(), ErrorData> {
        let Some(columns) = self.target.columns.as_deref() else {
            return Err(internal_error(
                "storing a result row",
                SinkError::ColumnsNotDeclared,
            ));
        };
        if cells.len() != columns.len() {
            return Err(store_error(&ResultStoreError::Invalid {
                reason: "row width differs from the column count",
            }));
        }
        let row_est = checked_row_bytes(cells)?;
        if let SinkMode::Buffered { rows, est_bytes } = &self.target.mode
            && !rows.is_empty()
            && (rows.len() >= MAX_BUFFER_ROWS
                || est_bytes
                    .checked_add(row_est)
                    .is_none_or(|next| next > self.target.buffer_byte_cap))
        {
            let (_, mut writer) = self.target.store.begin_and_drain(columns, rows)?;
            map_store(writer.push(cells))?;
            self.target.mode = SinkMode::Spilled(Box::new(writer));
            return Ok(());
        }

        match &mut self.target.mode {
            SinkMode::Spilled(writer) => map_store(writer.push(cells)),
            SinkMode::Buffered { rows, est_bytes } => {
                *est_bytes = match est_bytes.checked_add(row_est) {
                    Some(next) => next,
                    None => self.target.buffer_byte_cap,
                };
                rows.push(cells.to_vec());
                Ok(())
            }
        }
    }
}
