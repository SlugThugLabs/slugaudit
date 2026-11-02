//! File discovery, hashing, manifest comparison, and atomic revision
//! publish. Sync owns detecting and persisting what changed on disk; it
//! does not decide how tools query the result.
//!
//! Every sync is a full pass: the project is walked, every indexable file
//! is hashed, and only the files whose content differs from the stored row
//! are re-parsed and rewritten. The built-in content check
//! (`verify_stored`) additionally proves that each stored `content` still
//! hashes to its stored `content_hash`, so an edit made directly to the
//! database is repaired from the source on the next call.

mod analyze;
mod analyze_results;
mod analyze_seam;
mod classify;
mod discard_notice;
mod discovery;
mod evidence_limits;
mod file_pool;
mod hash;
mod hash_batch;
mod hash_io;
mod manager;
mod manager_meta;
mod manifest;
mod publish;
mod publish_attempt;
mod publish_cas;
mod publish_diff;
mod publish_log;
mod publish_sample;
mod race_hook;
mod recovery_log;
mod result_stores;
mod revalidate;
mod revision;
mod revision_edges;
mod revision_files;
mod revision_skipped;
mod sample;
mod sample_batch;
mod verify_stored;

pub use discard_notice::DatabaseDiscard;
pub use discovery::{DiscoveredFile, DiscoveryError, FileKind, SkippedFile, discover};
pub use hash::{HashError, aggregate_manifest_hash, hash_bytes, hash_file};
pub use manager::{SourceSyncManager, SyncedProject};
pub(crate) use manager::{map_activation_error, map_process_limits_error, session_findings_for};
pub use manifest::{ChangeStatus, FileChange, compare};
pub use publish::{PublishError, PublishReport, publish};
pub(crate) use publish_log::failure_context as publish_failure_context;
pub use result_stores::ResultStoreSlot;
pub use revision::{FileRecord, RevisionError};
pub(crate) use revision::{current_revision_id, current_revision_row};

#[cfg(test)]
#[path = "timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
pub(super) use publish_cas::publish_with_limits;
