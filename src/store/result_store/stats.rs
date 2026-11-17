//! A snapshot of the store's state and decisions, for `health`.

use super::error::ResultStoreError;
use super::owner::SweepOutcome;
use super::registry::{Registry, ResultClass};

/// Results and bytes held in one class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassStats {
    /// Number of results.
    pub results: usize,
    /// Their measured bytes.
    pub bytes: u64,
}

/// What the store holds and has decided so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreStats {
    /// Active results (never reclaimed).
    pub active: ClassStats,
    /// Idle results (reclaimable second).
    pub idle: ClassStats,
    /// Delivered results (reclaimable first).
    pub delivered: ClassStats,
    /// `max_result_store_bytes`.
    pub budget_bytes: u64,
    /// Sum of admitted bytes; never above the budget.
    pub admitted_bytes: u64,
    /// Delivered results reclaimed so far.
    pub reclaimed_delivered: u64,
    /// Idle results reclaimed so far.
    pub reclaimed_idle: u64,
    /// New results refused with `Full`.
    pub refused_full: u64,
    /// New results refused with `TooLarge`.
    pub refused_too_large: u64,
    /// Cursors refused as unknown.
    pub unknown_cursors: u64,
    /// Cursors refused as expired.
    pub expired_cursors: u64,
    /// What the startup sweep did.
    pub sweep: SweepOutcome,
}

impl Registry {
    pub(super) fn snapshot(
        &self,
        budget_bytes: u64,
        now: i64,
        grace_secs: u64,
        sweep: &SweepOutcome,
    ) -> Result<StoreStats, ResultStoreError> {
        let mut stats = StoreStats {
            budget_bytes,
            admitted_bytes: self.admitted_bytes,
            reclaimed_delivered: self.counters.reclaimed_delivered,
            reclaimed_idle: self.counters.reclaimed_idle,
            refused_full: self.counters.refused_full,
            refused_too_large: self.counters.refused_too_large,
            unknown_cursors: self.counters.unknown_cursors,
            expired_cursors: self.counters.expired_cursors,
            // KEEP clone: stats borrows &SweepOutcome and returns owned StoreStats
            sweep: sweep.clone(),
            ..StoreStats::default()
        };
        for entry in self.entries.values() {
            let slot = match entry.class(now, grace_secs) {
                ResultClass::Active => &mut stats.active,
                ResultClass::Idle => &mut stats.idle,
                ResultClass::Delivered => &mut stats.delivered,
            };
            // The class totals sum the entries that `admitted_bytes` already
            // counts, so an overflow is inconsistent bookkeeping.
            let (Some(results), Some(bytes)) = (
                slot.results.checked_add(1),
                slot.bytes.checked_add(entry.bytes),
            ) else {
                return Err(ResultStoreError::corrupt("result class totals overflow"));
            };
            slot.results = results;
            slot.bytes = bytes;
        }
        Ok(stats)
    }
}
