//! Pass 2 selection and check: chooses which files Pass 2 reads and verifies
//! the samples still match the Pass 1 stream hashes.

use super::discovery;
use super::manifest::{ChangeStatus, FileChange};
use super::publish::PublishError;
use super::sample;
use crate::model::SourceIdentity;
use std::collections::HashSet;

/// Picks the discovered files Pass 2 must read: every hashed file when the
/// parser version changed, otherwise only the hashed files the diff marks as
/// added or modified.
pub(super) fn files_to_sample(
    discovered: &[discovery::DiscoveredFile],
    identities: &[SourceIdentity],
    changes: &[FileChange],
    parser_version_changed: bool,
) -> Vec<discovery::DiscoveredFile> {
    let hashed_paths: HashSet<&str> = identities
        .iter()
        .map(|id| id.relative_path.as_str())
        .collect();
    if parser_version_changed {
        return discovered
            .iter()
            .filter(|f| hashed_paths.contains(f.relative_path.as_str()))
            .cloned()
            .collect();
    }
    let changed_set: HashSet<&str> = changes
        .iter()
        .filter(|c| c.status != ChangeStatus::Deleted)
        .map(|c| c.relative_path.as_str())
        .collect();
    discovered
        .iter()
        .filter(|f| {
            changed_set.contains(f.relative_path.as_str())
                && hashed_paths.contains(f.relative_path.as_str())
        })
        .cloned()
        .collect()
}

/// Validates that no sampled file was modified between Pass 1 stream-hashing
/// and Pass 2 selective sampling. If content changed, the attempt aborts so
/// `publish_cas` retries rather than commit a mismatched manifest hash.
pub(super) fn check_samples_match_pass1(
    samples: &[sample::Sample],
    identities: &[SourceIdentity],
) -> Result<(), PublishError> {
    let pass1_hashes: std::collections::HashMap<&str, &str> = identities
        .iter()
        .map(|id| (id.relative_path.as_str(), id.content_hash.as_str()))
        .collect();
    for sample in samples {
        if let Some(&p1_hash) = pass1_hashes.get(sample.relative_path.as_str())
            && sample.identity.content_hash != p1_hash
        {
            return Err(PublishError::ChangedDuringSample {
                path: sample.relative_path.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "publish_sample_tests.rs"]
mod tests;
