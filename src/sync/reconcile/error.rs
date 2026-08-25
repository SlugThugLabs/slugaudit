//! The [`ReconcileError`] type shared by the pipeline and the barrier
//! loop. Kept in its own module so both callers can name it without
//! importing the whole reconcile surface.

use crate::sync::discovery::DiscoveryError;
use crate::sync::hash::HashError;
use crate::sync::revision::RevisionError;
use crate::sync::sample::SampleError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error(transparent)]
    Sample(#[from] SampleError),
    #[error(transparent)]
    Revision(#[from] RevisionError),
    /// Barrier sync hit its iteration cap. The watcher is being marked
    /// `Desynced` so the next sync call falls back to a full verification
    /// rather than continuing to drain an endless stream of racing events.
    /// Reaching this is a clear signal of a pathological producer
    /// (editor saving faster than reconcile completes, fsmonitor firing
    /// repeatedly, etc.) — preferable to looping forever and exhausting
    /// memory.
    #[error(
        "barrier sync hit the {iterations}-iteration cap, watching is marked Desynced \
         and the next call will do a full verification"
    )]
    BarrierCapExceeded { iterations: u32 },
    /// `path` is a pre-formatted note naming the file being processed when
    /// the budget tripped (e.g. `" while processing src/gen/big.rs"`), or
    /// empty when the tripping site has no single file in hand (the
    /// barrier loop). Kept pre-formatted so the error reads cleanly.
    #[error("reconcile exceeded its wall-clock time budget after {elapsed_ms} ms{path}")]
    TimeBudgetExceeded { path: String, elapsed_ms: u128 },
}
