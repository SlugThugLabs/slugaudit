//! File discovery, hashing, manifest comparison, and atomic revision
//! publish. Sync owns detecting and persisting what changed on disk; it
//! does not decide how tools query the result.

mod analyze;
mod analyze_results;
mod analyze_seam;
mod classify;
mod discovery;
mod hash;
mod hash_io;
mod manager;
mod manager_health_support;
mod manager_meta;
mod manifest;
mod publish;
mod publish_attempt;
mod publish_cas;
mod publish_diff;
mod publish_log;
mod race_hook;
mod reconcile;
mod result_stores;
mod revalidate;
mod revision;
mod revision_edges;
mod revision_files;
mod revision_skipped;
mod revision_support;
mod sample;
mod sample_batch;
mod sweep;
mod sweep_reconcile;

pub use discovery::{DiscoveredFile, DiscoveryError, FileKind, SkippedFile, discover};
pub use hash::{HashError, aggregate_manifest_hash, hash_bytes, hash_file};
pub(crate) use manager::map_activation_error;
pub use manager::{SourceSyncManager, SyncedProject};
pub use manifest::{ChangeStatus, FileChange, compare};
pub use publish::{PublishError, PublishReport, publish};
pub use result_stores::ResultStoreSlot;
pub use revision::{FileRecord, RevisionError};

#[cfg(test)]
#[path = "timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
pub(super) use publish_cas::publish_with_limits;
#[cfg(test)]
pub(super) use reconcile::{
    ReconcileOptions, reconcile_dirty_paths_with_deadline, sync_with_barrier_with_deadline,
};
