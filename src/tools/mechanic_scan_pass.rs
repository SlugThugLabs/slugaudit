//! The rows one scan pass retains for caching, under one budget shared by
//! every entry of the pass.

use super::mechanic_scan_cached::index_out_of_range;
use crate::project::CachedRow;
use rmcp::ErrorData;

/// Rows being retained for one entry while the pass runs.
#[derive(Default)]
struct Retained {
    rows: Vec<CachedRow>,
    bytes: usize,
    overflowed: bool,
}

/// The rows one scan pass retains for caching, with one budget shared by
/// every entry of the pass: a call selecting many uncached entries holds at
/// most the cache budget in total, not the budget per entry. An entry whose
/// row would take the pass past the budget gives up its rows (and releases
/// their bytes); it is still counted, just not cached.
pub(super) struct Pass {
    budget: usize,
    bytes: usize,
    entries: Vec<Retained>,
}

impl Pass {
    pub(super) fn new(budget: usize, entries: usize) -> Self {
        Self {
            budget,
            bytes: 0,
            entries: (0..entries).map(|_| Retained::default()).collect(),
        }
    }

    /// Offers one row of entry `index` for retention.
    ///
    /// # Errors
    ///
    /// An internal error when `index` names no entry of the pass.
    pub(super) fn offer(&mut self, index: usize, row: CachedRow) -> Result<(), ErrorData> {
        let entries = self.entries.len();
        let keep = self
            .entries
            .get_mut(index)
            .ok_or_else(|| index_out_of_range("pass entries", index, entries))?;
        if keep.overflowed {
            return Ok(());
        }
        let size = row.estimated_bytes();
        match self.bytes.checked_add(size) {
            // A sum past `usize::MAX` is over any budget.
            Some(total) if total <= self.budget => {
                keep.bytes = keep
                    .bytes
                    .checked_add(size)
                    .ok_or_else(accounting_overflow)?;
                self.bytes = total;
                keep.rows.push(row);
            }
            _ => {
                self.bytes = self
                    .bytes
                    .checked_sub(keep.bytes)
                    .ok_or_else(accounting_overflow)?;
                keep.bytes = 0;
                keep.overflowed = true;
                keep.rows = Vec::new();
            }
        }
        Ok(())
    }

    /// Estimated bytes of the rows retained right now, over all entries
    /// (the seam the retention test reads).
    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.bytes
    }

    /// Entry `index`'s rows, or `None` when it gave up retaining them.
    ///
    /// # Errors
    ///
    /// An internal error when the pass's byte accounting is inconsistent.
    pub(super) fn take(&mut self, index: usize) -> Result<Option<Vec<CachedRow>>, ErrorData> {
        let Some(keep) = self.entries.get_mut(index) else {
            return Ok(None);
        };
        if keep.overflowed {
            return Ok(None);
        }
        self.bytes = self
            .bytes
            .checked_sub(keep.bytes)
            .ok_or_else(accounting_overflow)?;
        keep.bytes = 0;
        Ok(Some(std::mem::take(&mut keep.rows)))
    }
}

/// The internal error for a retained-byte count that does not add up: an
/// invariant violation, logged once here and failing the call.
fn accounting_overflow() -> ErrorData {
    super::context::internal_error(
        "retaining scan rows",
        "the retained byte count is inconsistent",
    )
}
