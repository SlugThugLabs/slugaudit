//! Pass 1 of a full sync: stream-hash every discovered file on the shared
//! worker pool ([`super::file_pool`]).
//!
//! Hashes files on disk in fixed 64 KB chunks ([`hash_file`]) to produce
//! [`SourceIdentity`] without holding entire file texts in memory.

use super::discovery::{DiscoveredFile, SkippedFile};
use super::file_pool::{self, Pass};
use super::hash::{HashError, hash_file};
use super::publish::PublishError;
use super::revision_skipped::SKIP_REASON_OVERSIZE;
use crate::model::SourceIdentity;
use crate::util::Deadline;

/// The hashing pass, as its log lines and time-budget error name it.
const HASH_PASS: Pass = Pass {
    operation: "hash_batch",
    activity: "hashing",
};

/// One file's Pass 1 outcome.
enum Hashed {
    /// The file's content identity.
    Identity(SourceIdentity),
    /// The file is over the per-file ceiling and becomes a skipped row.
    Skipped(SkippedFile),
}

/// Hashes every discovered file, in discovered order. A file over
/// `max_file_bytes` is skipped with an `oversize` reason instead of failing
/// the pass.
///
/// # Errors
///
/// [`PublishError::ChangedDuringSample`] for a file deleted since the walk,
/// the read error for any other unreadable file, or a pool failure (see
/// [`file_pool::run`]).
pub(super) fn hash_all_with_deadline(
    discovered: &[DiscoveredFile],
    max_file_bytes: u64,
    deadline: &Deadline,
) -> Result<(Vec<SourceIdentity>, Vec<SkippedFile>), PublishError> {
    let outcomes = file_pool::run(discovered, deadline, HASH_PASS, |_, file| {
        hash_one(file, max_file_bytes)
    })?;
    let mut identities = Vec::with_capacity(outcomes.len());
    let mut skips = Vec::new();
    for outcome in outcomes {
        match outcome {
            Hashed::Identity(identity) => identities.push(identity),
            Hashed::Skipped(skip) => skips.push(skip),
        }
    }
    Ok((identities, skips))
}

/// Hashes one file: its identity, an oversize skip, or the failure.
fn hash_one(file: &DiscoveredFile, max_file_bytes: u64) -> Result<Hashed, PublishError> {
    match hash_file(&file.relative_path, &file.absolute_path, max_file_bytes) {
        Ok(identity) => Ok(Hashed::Identity(identity)),
        Err(error @ HashError::TooLarge { .. }) => Ok(Hashed::Skipped(SkippedFile {
            absolute_path: file.absolute_path.clone(),
            reason: error.to_string(),
            code: SKIP_REASON_OVERSIZE,
        })),
        Err(HashError::Read { path, source }) => Err(PublishError::read_after_walk(
            &file.relative_path,
            path,
            source,
        )),
    }
}

#[cfg(test)]
#[path = "hash_batch_tests.rs"]
mod tests;
