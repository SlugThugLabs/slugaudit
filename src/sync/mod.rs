//! File discovery, hashing, manifest comparison, and atomic revision
//! publish. Sync owns detecting and persisting what changed on disk; it
//! does not decide how tools query the result.

mod analyze;
mod discovery;
mod hash;
mod manager;
mod manager_meta;
mod manifest;
mod publish;
mod publish_attempt;
mod publish_cas;
mod publish_diff;
mod publish_log;
mod race_hook;
mod reconcile;
mod revalidate;
mod revision;
mod revision_edges;
mod sample;

pub use discovery::{DiscoveredFile, DiscoveryError, FileKind, SkippedFile, discover};
pub use hash::{HashError, aggregate_manifest_hash, hash_bytes, hash_file};
pub use manager::{SourceSyncManager, SyncedProject};
pub use manifest::{ChangeStatus, FileChange, compare};
pub use publish::{PublishError, PublishReport, publish};
pub use reconcile::{ReconcileError, ReconcileReport, reconcile_dirty_paths, sync_with_barrier};
pub use revision::{FileRecord, RevisionError};

#[cfg(test)]
#[path = "timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
pub(super) use publish_cas::publish_with_limits;
#[cfg(test)]
pub(super) use reconcile::{reconcile_dirty_paths_with_deadline, sync_with_barrier_with_deadline};
