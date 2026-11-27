//! Pass 1 of a full sync: single-open parallel binary sniff + BLAKE3 stream
//! hash on the shared worker pool ([`super::file_pool`]).
//!
//! Hashes files on disk in fixed 64 KiB thread-local chunks without holding
//! entire file texts in memory or opening each file twice.

use super::discovery::{DiscoveredFile, DiscoveryError, SkippedFile, WalkedFile, is_vanished};
use super::file_pool::{self, Pass};
use super::hash::{HashError, sniff_and_hash_file};
use super::publish::PublishError;
use super::revision::{SKIP_REASON_OVERSIZE, SKIP_REASON_UNREADABLE};
use crate::model::SourceIdentity;
use crate::util::Deadline;

/// The hashing pass, as its log lines and time-budget error name it.
const HASH_PASS: Pass = Pass {
    operation: "hash_batch",
    activity: "hashing",
};

/// One walked file's single-pass sniff + hash outcome.
enum WalkHashed {
    /// Binary-classified discovered file paired with its BLAKE3 identity.
    Classified(DiscoveredFile, SourceIdentity),
    /// Unreadable or oversize file recorded as a skipped row.
    Skipped(SkippedFile),
}

/// Output of Pass 1 single-open binary sniffing and BLAKE3 stream hashing.
#[cfg_attr(test, derive(Debug))]
pub(crate) struct SniffedWalk {
    pub(crate) discovered: Vec<DiscoveredFile>,
    pub(crate) identities: Vec<SourceIdentity>,
    pub(crate) skips: Vec<SkippedFile>,
}

/// Opens each walked regular file once on the bounded worker pool to both
/// classify binary vs. text and stream-hash its content up to `max_file_bytes`.
///
/// Unreadable files (e.g. `PermissionDenied`, or a non-regular file swapped in
/// after the walk) become `unreadable` skips and oversized files become
/// `oversize` skips; a file that vanished (`NotFound`) since the walk returns
/// [`PublishError::ChangedDuringSample`] so `publish` retries.
pub(super) fn sniff_and_hash_walked(
    walked: &[WalkedFile],
    max_file_bytes: u64,
    deadline: &Deadline,
) -> Result<SniffedWalk, PublishError> {
    let outcomes = file_pool::run(walked, deadline, HASH_PASS, |_, file| {
        sniff_and_hash_one(file, max_file_bytes)
    })?;
    let mut discovered = Vec::with_capacity(outcomes.len());
    let mut identities = Vec::with_capacity(outcomes.len());
    let mut skips = Vec::new();
    for outcome in outcomes {
        match outcome {
            WalkHashed::Classified(file, identity) => {
                discovered.push(file);
                identities.push(identity);
            }
            WalkHashed::Skipped(skip) => skips.push(skip),
        }
    }
    Ok(SniffedWalk {
        discovered,
        identities,
        skips,
    })
}

fn sniff_and_hash_one(file: &WalkedFile, max_file_bytes: u64) -> Result<WalkHashed, PublishError> {
    match sniff_and_hash_file(&file.relative_path, &file.absolute_path, max_file_bytes) {
        Ok((kind, identity)) => Ok(WalkHashed::Classified(
            DiscoveredFile {
                relative_path: file.relative_path.clone(),
                absolute_path: file.absolute_path.clone(),
                kind,
            },
            identity,
        )),
        Err(error @ HashError::TooLarge { .. }) => Ok(WalkHashed::Skipped(SkippedFile {
            absolute_path: file.absolute_path.clone(),
            reason: error.to_string(),
            code: SKIP_REASON_OVERSIZE,
        })),
        Err(HashError::Read { source, .. }) if is_vanished(&source) => {
            Err(PublishError::ChangedDuringSample {
                path: file.relative_path.clone(),
            })
        }
        Err(HashError::Read { path, source }) => Ok(WalkHashed::Skipped(SkippedFile {
            absolute_path: file.absolute_path.clone(),
            reason: DiscoveryError::Read { path, source }.to_string(),
            code: SKIP_REASON_UNREADABLE,
        })),
    }
}

#[cfg(test)]
#[path = "hash_batch_tests.rs"]
mod tests;
