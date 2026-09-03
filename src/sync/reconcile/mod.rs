//! Reconciliation logic for SlugAudit's watch-based sync.
//!
//! When the filesystem watcher reports dirty or deleted paths, this module
//! reconciles them against the database: hashing dirty files, comparing
//! with stored hashes, and re-indexing only files that actually changed.
//! The barrier synchronization loop ensures no events are lost even if
//! new events arrive during reconciliation.
//!
//! Split by concern so each piece stays small and independently readable:
//! [`error`] (the error type), [`report`] (the reconcile report),
//! [`options`] (budget + ignore-rule policy), [`pipeline`] (the per-path
//! dirty-file loop), [`barrier`] (the event-barrier synchronization loop),
//! and [`queries`] (the SQL the pipeline reads). This `mod.rs` only
//! re-exports the public surface.

mod barrier;
mod error;
mod options;
mod pipeline;
mod queries;
mod report;

/// Maximum number of barrier-sync iterations before giving up. A
/// pathological editor that emits events faster than reconciliation
/// completes would otherwise loop forever, exhausting memory and never
/// returning to the caller. With this cap, the watcher is marked
/// `Desynced` and the next sync call falls back to a full verification.
/// Declared here (rather than in [`barrier`]) so tests attached to this
/// module can name it without reaching into a private submodule.
pub(crate) const MAX_BARRIER_LOOPS: u32 = 16;

#[cfg(test)]
pub(crate) use barrier::sync_with_barrier;
pub(crate) use barrier::sync_with_barrier_with_deadline;
pub(crate) use error::ReconcileError;
pub(crate) use options::ReconcileOptions;
#[cfg(test)]
pub(crate) use pipeline::reconcile_dirty_paths;
pub(crate) use pipeline::reconcile_dirty_paths_with_deadline;
pub(crate) use report::ReconcileReport;

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reconcile_binary_tests.rs"]
mod binary_tests;

#[cfg(test)]
#[path = "reconcile_ignore_tests.rs"]
mod ignore_tests;
