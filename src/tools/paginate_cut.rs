//! Row-cutting state machine (`RowCutter`), byte-budget policy, raw-row
//! `StoredRow` normalization, and [`ServedRows`] assembly for `paginate`.

use super::context::internal_error;
use super::page::{Completeness, Page, PageLimit};
use crate::store::{Cell, NonFinite, StoredRow};
use rmcp::ErrorData;

/// Decision returned by a tool's single-decode row-cut closure in `paginate_rows`.
pub(crate) enum CutStep<R> {
    /// Keep `row` (whose serialized JSON size is `bytes`) on this page.
    Keep { row: R, bytes: usize },
    /// Stop before this row; `limit` names a non-byte cap (`Limit`, `MatchCap`),
    /// or `None` when the page byte budget (`PageBytes` / `Ceiling`) stopped it.
    Stop { limit: Option<PageLimit> },
}

/// Helper for tools whose per-row cut is the JSON byte budget (`page_bytes`).
pub(in crate::tools) fn byte_cut_step<R>(
    row: R,
    bytes: usize,
    taken: usize,
    bytes_so_far: usize,
    budget: usize,
) -> CutStep<R> {
    if taken == 0 {
        return CutStep::Keep { row, bytes };
    }
    match bytes_so_far
        .checked_add(bytes)
        .filter(|total| *total <= budget)
    {
        Some(_) => CutStep::Keep { row, bytes },
        None => CutStep::Stop { limit: None },
    }
}

/// One served page of decoded items `R` from `paginate_rows`.
pub(crate) struct ServedRows<R, E> {
    pub(crate) rows: Vec<R>,
    pub(crate) page: Page,
    pub(crate) meta: E,
    #[cfg(test)]
    pub(super) columns: Vec<String>,
    #[cfg(test)]
    pub(super) at_ceiling: bool,
}

/// Context passed to a page-cut closure on both fresh requests and continuations.
pub struct PageContext<'a, E> {
    /// Effective byte budget of this page.
    pub page_bytes: usize,
    /// Whether `page_bytes` is the ceiling the result was made under.
    pub at_ceiling: bool,
    /// Column names stored with the result.
    pub columns: &'a [String],
    /// Tool-level extras the producer reported, as stored.
    pub extra: &'a E,
}

/// Parameters assembled into a [`Page`] + [`ServedRows`] once `RowCutter` finishes.
pub(super) struct ServedMeta<E> {
    pub(super) total_rows: u64,
    pub(super) cursor: Option<String>,
    pub(super) completeness: Completeness,
    pub(super) page_bytes: usize,
    pub(super) at_ceiling: bool,
    pub(super) revision_id: String,
    pub(super) current_disk_revision_id: Option<String>,
    pub(super) database_discard: Option<crate::sync::DatabaseDiscard>,
    pub(super) extra: E,
    #[cfg(test)]
    pub(super) columns: Vec<String>,
}

/// Shared row-cutting state machine used by both in-memory Page 1 cuts and
/// `ResultStore` page reads.
pub(super) struct RowCutter<R, C> {
    cut: C,
    max_rows: Option<usize>,
    rows: Vec<R>,
    bytes_used: usize,
    stop_limit: Option<PageLimit>,
}

impl<R, C> RowCutter<R, C>
where
    C: FnMut(&StoredRow, usize, usize) -> Result<CutStep<R>, ErrorData>,
{
    pub(super) fn new(cut: C, max_rows: Option<usize>) -> Self {
        Self {
            cut,
            max_rows,
            rows: Vec::new(),
            bytes_used: 0,
            stop_limit: None,
        }
    }

    pub(super) fn accept(&mut self, row: &StoredRow) -> Result<bool, ErrorData> {
        let taken = self.rows.len();
        if taken > 0 && self.max_rows.is_some_and(|cap| taken >= cap.max(1)) {
            self.stop_limit = Some(PageLimit::MatchCap);
            return Ok(false);
        }
        match (self.cut)(row, taken, self.bytes_used)? {
            CutStep::Keep { row: item, bytes } => {
                self.bytes_used = self
                    .bytes_used
                    .checked_add(bytes)
                    .ok_or_else(|| internal_error("measuring page bytes", "byte sum overflowed"))?;
                self.rows.push(item);
                Ok(true)
            }
            CutStep::Stop { limit } if taken > 0 => {
                self.stop_limit = limit;
                Ok(false)
            }
            CutStep::Stop { .. } => Err(internal_error(
                "cutting the first row of a page",
                "the first row of a page must always be kept",
            )),
        }
    }

    pub(super) fn into_served<E>(self, meta: ServedMeta<E>) -> ServedRows<R, E> {
        let more_pages = meta.cursor.is_some();
        let partial_limit = match &meta.completeness {
            crate::tools::page::Completeness::Partial { limited_by, .. } => Some(*limited_by),
            crate::tools::page::Completeness::Complete => None,
        };
        let complete = !more_pages && partial_limit.is_none();
        let limited_by = if more_pages {
            compute_limited_by(false, self.stop_limit, meta.at_ceiling)
        } else if let Some(limit) = partial_limit {
            limit
        } else {
            PageLimit::None
        };
        let page = Page {
            returned: self.rows.len(),
            total: Some(meta.total_rows),
            complete,
            cursor: meta.cursor,
            completeness: Some(meta.completeness),
            page_bytes: meta.page_bytes,
            bytes_used: self.bytes_used,
            limited_by,
            revision_id: meta.revision_id,
            current_disk_revision_id: meta.current_disk_revision_id,
            database_discard: meta.database_discard,
        };
        ServedRows {
            rows: self.rows,
            page,
            meta: meta.extra,
            #[cfg(test)]
            columns: meta.columns,
            #[cfg(test)]
            at_ceiling: meta.at_ceiling,
        }
    }
}

pub(in crate::tools) fn compute_limited_by(
    complete: bool,
    stop_limit: Option<PageLimit>,
    at_ceiling: bool,
) -> PageLimit {
    if complete {
        return PageLimit::None;
    }
    match stop_limit {
        None | Some(PageLimit::PageBytes) => {
            if at_ceiling {
                PageLimit::Ceiling
            } else {
                PageLimit::PageBytes
            }
        }
        Some(limit) => limit,
    }
}

/// Converts a raw buffered row into the exact [`StoredRow`] layout `ResultStore::read_page` yields.
pub(super) fn stored_row_from_raw(ord: u64, raw: &[Cell]) -> StoredRow {
    let mut cells = Vec::with_capacity(raw.len());
    let mut nonfinite = Vec::new();
    for (index, cell) in raw.iter().enumerate() {
        match cell {
            Cell::Real(value) if value.is_nan() => {
                nonfinite.push((index, NonFinite::NaN));
                cells.push(Cell::Null);
            }
            Cell::Real(value) => {
                if value.is_infinite() {
                    let mark = if value.is_sign_positive() {
                        NonFinite::Infinity
                    } else {
                        NonFinite::NegInfinity
                    };
                    nonfinite.push((index, mark));
                }
                cells.push(Cell::Real(*value));
            }
            other => cells.push(other.clone()),
        }
    }
    StoredRow {
        ord,
        cells,
        nonfinite,
    }
}
