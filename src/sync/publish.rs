use super::discovery;
use super::revision;
use super::sample;
use thiserror::Error;

pub use super::publish_cas::publish;

// Re-exports for the test modules, which are children of this module and
// access these items via `super::*`. The definitions live in their natural
// homes; this module just makes them reachable from tests. Gated behind
// `cfg(test)` because nothing in the library build uses them directly.
#[cfg(test)]
pub(super) use super::publish_cas::{MAX_CAS_RETRIES, is_retryable};
#[cfg(test)]
pub(super) use super::publish_diff::build_upserts_and_deletions;
#[cfg(test)]
pub(super) use super::race_hook;
#[cfg(test)]
pub(super) use super::revalidate::revalidate_unchanged_since_sample;
#[cfg(test)]
pub(super) use super::revision::RevisionError;
#[cfg(test)]
pub(super) use super::sample::sample_all;
#[cfg(test)]
pub(super) use crate::model::ResourceLimits;
#[cfg(test)]
pub(super) use rusqlite::Connection;
#[cfg(test)]
use std::path::Path;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error(transparent)]
    Discovery(#[from] discovery::DiscoveryError),
    #[error(transparent)]
    Sample(#[from] sample::SampleError),
    #[error(transparent)]
    Revision(#[from] revision::RevisionError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "import would load {total} bytes across sampled files, exceeding the {limit}-byte ceiling"
    )]
    ImportTooLarge { total: u64, limit: u64 },
    /// A file's on-disk content changed between being sampled and the
    /// publish transaction that would have written it as current. Caught by
    /// `revalidate_unchanged_since_sample` immediately before the write;
    /// the caller retries with an entirely fresh sample rather than
    /// publishing a revision built from stale bytes.
    #[error("{path} changed on disk after being sampled; retrying with a fresh sample")]
    ChangedDuringSample { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub revision_id: String,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub unchanged: usize,
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "publish_race_tests.rs"]
mod race_tests;

#[cfg(test)]
#[path = "publish_edges_tests.rs"]
mod edges_tests;

#[cfg(test)]
#[path = "publish_acceptance_tests.rs"]
mod acceptance_tests;

#[cfg(test)]
#[path = "publish_mutation_tests.rs"]
mod mutation_tests;
