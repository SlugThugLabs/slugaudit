//! File discovery, single-pass binary sniffing + BLAKE3 hashing, manifest
//! diffing, uncontended Pass 2 staging, and atomic CAS revision publishing.
//!
//! Every sync walks the project, stream-hashes every indexable regular file on
//! disk, compares against `files.content_hash` in SQLite, and re-parses and
//! rewrites only changed or newly added rows without re-reading stored
//! `files.content`.

mod analyze;
mod classify;
mod discovery;
mod file_pool;
mod hash;
mod hash_batch;
mod manager;
mod manifest;
mod project_meta;
mod publish;
mod publish_attempt;
mod publish_diff;
mod publish_log;
mod race_hook;
mod result_stores;
mod revision;
mod revision_cargo_lib_stems;
mod revision_edge_invalidation;
mod revision_edge_match;
mod revision_edges;
mod revision_graph_context;
mod revision_path_state;
mod revision_rust_dependents;
mod revision_rust_meta;
mod revision_rust_path_stems;
mod revision_rust_shadowing;
mod revision_rust_siblings;
mod revision_rust_targets;
mod revision_super_scope;
mod rust_super_paths;
mod sample;
mod sample_batch;
mod staging;

#[cfg(test)]
pub(crate) use discovery::discover_and_hash;
pub use discovery::{DiscoveredFile, DiscoveryError, FileKind, SkippedFile};
pub use hash::{HashError, hash_bytes, hash_file};
pub use manager::{DatabaseDiscard, SourceSyncManager, SyncedProject};
pub(crate) use manager::{map_activation_error, map_process_limits_error, session_findings_for};
pub use manifest::{ChangeStatus, FileChange, compare};
pub(crate) use project_meta::project_instance_id;
#[cfg(test)]
pub(super) use publish::publish_with_limits;
pub use publish::{PublishError, PublishReport, publish};
#[cfg(test)]
pub(crate) use publish_diff::add_available_grammar_retries;
pub use result_stores::ResultStoreSlot;
pub use revision::{FileRecord, RevisionError};
pub(crate) use revision::{current_revision_id, current_revision_row};

#[cfg(test)]
#[path = "timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
pub(crate) mod test_support;
