//! File discovery, hashing, manifest comparison, and atomic revision
//! publish. Sync owns detecting and persisting what changed on disk; it
//! does not decide how tools query the result.

mod analyze;
mod discovery;
mod hash;
mod manifest;
mod publish;
mod publish_diff;
mod race_hook;
mod revision;
mod revision_edges;
mod sample;

pub use discovery::{DiscoveredFile, DiscoveryError, FileKind, discover};
pub use hash::{HashError, aggregate_manifest_hash, hash_bytes, hash_file};
pub use manifest::{ChangeStatus, FileChange, compare};
pub use publish::{PublishError, PublishReport, publish};
pub use revision::{FileRecord, RevisionError};
