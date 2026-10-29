//! The result store's share of `health` detail: what paged results the
//! server holds and which decisions it has made about them.

use crate::store::{ClassStats, StoreStats, SweepOutcome};
use crate::sync;
use serde::Serialize;

/// Results and bytes held in one retention class.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ResultClassState {
    /// Number of results in the class.
    pub results: usize,
    /// Their measured bytes.
    pub bytes: u64,
}

impl From<ClassStats> for ResultClassState {
    fn from(stats: ClassStats) -> Self {
        Self {
            results: stats.results,
            bytes: stats.bytes,
        }
    }
}

/// What the startup sweep of leftover result files did.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ResultSweepState {
    /// Files removed (dead owners' databases and lock files, orphans).
    pub removed_files: usize,
    /// Sessions another live process holds; their files were left.
    pub live_owners_skipped: usize,
    /// Operations that failed while the sweep carried on.
    pub errors: usize,
    /// The first failure's message; null when none failed.
    pub first_error: Option<String>,
}

impl From<SweepOutcome> for ResultSweepState {
    fn from(sweep: SweepOutcome) -> Self {
        Self {
            removed_files: sweep.removed_files,
            live_owners_skipped: sweep.live_owners_skipped,
            errors: sweep.errors,
            first_error: sweep.first_error,
        }
    }
}

/// The result store, as `health` reports it.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ResultStoreState {
    /// Unfinished results served within the grace period; never reclaimed.
    pub active: ResultClassState,
    /// Unfinished results older than the grace period; reclaimed second.
    pub idle: ResultClassState,
    /// Fully read results kept for a final-page retry; reclaimed first.
    pub delivered: ResultClassState,
    /// `max_result_store_bytes`: the most the store admits.
    pub budget_bytes: u64,
    /// Bytes of all admitted results; never above the budget.
    pub admitted_bytes: u64,
    /// Delivered results reclaimed since the server started.
    pub reclaimed_delivered: u64,
    /// Idle results reclaimed since the server started.
    pub reclaimed_idle: u64,
    /// New results refused with `result_store_full`.
    pub refused_full: u64,
    /// New results refused with `result_too_large`.
    pub refused_too_large: u64,
    /// Cursors refused as `unknown_cursor`.
    pub unknown_cursors: u64,
    /// Cursors refused as `result_expired`.
    pub expired_cursors: u64,
    /// What the startup sweep did.
    pub sweep: ResultSweepState,
}

impl From<StoreStats> for ResultStoreState {
    fn from(stats: StoreStats) -> Self {
        Self {
            active: stats.active.into(),
            idle: stats.idle.into(),
            delivered: stats.delivered.into(),
            budget_bytes: stats.budget_bytes,
            admitted_bytes: stats.admitted_bytes,
            reclaimed_delivered: stats.reclaimed_delivered,
            reclaimed_idle: stats.reclaimed_idle,
            refused_full: stats.refused_full,
            refused_too_large: stats.refused_too_large,
            unknown_cursors: stats.unknown_cursors,
            expired_cursors: stats.expired_cursors,
            sweep: stats.sweep.into(),
        }
    }
}

/// The store's state, or `None` while no paged result was ever made in this
/// process (the store opens on the first one).
pub(super) fn snapshot(manager: &sync::SourceSyncManager) -> Option<ResultStoreState> {
    manager
        .result_stores()
        .current()
        .map(|store| store.stats().into())
}
