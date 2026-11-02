//! Re-validates that files haven't changed on disk between sampling and the
//! publish transaction that would record them. A file whose content changed
//! in that window must not be published as if it were stable — the caller
//! retries with a fresh sample instead.

use super::discovery::DiscoveredFile;
use super::file_pool::time_budget_error;
use super::publish::PublishError;
use super::revision::FileRecord;
use super::sample::{SampleError, sample_file};
use crate::model::ResourceLimits;
use crate::util::Deadline;
use std::collections::HashMap;

/// Re-samples every file this function is about to write and aborts if any
/// no longer matches what was recorded during the original sample. This
/// closes the TOCTOU gap between "we read this file" and "we told the
/// database this is the file's current state" as tightly as is achievable
/// without filesystem-level locking, which ordinary editors (atomic
/// rename-on-save in particular) would defeat anyway.
///
/// # Errors
///
/// Returns `ChangedDuringSample` for the first file found to have changed,
/// so the caller can retry with an entirely fresh sample.
pub(super) fn revalidate_unchanged_since_sample(
    discovered: &[DiscoveredFile],
    upserts: &[FileRecord],
    limits: &ResourceLimits,
    deadline: &Deadline,
) -> Result<(), PublishError> {
    let by_path: HashMap<&str, &DiscoveredFile> = discovered
        .iter()
        .map(|file| (file.relative_path.as_str(), file))
        .collect();
    for upsert in upserts {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(time_budget_error(
                "processing",
                &upsert.relative_path,
                elapsed,
            ));
        }
        let unchanged = match by_path.get(upsert.relative_path.as_str()) {
            Some(file) => still_matches(file, upsert, limits)?,
            None => false,
        };
        if !unchanged {
            return Err(PublishError::ChangedDuringSample {
                path: upsert.relative_path.clone(),
            });
        }
    }
    Ok(())
}

/// Whether `file` still hashes to what `upsert` recorded. Every upsert was
/// sampled as an indexable file, so a file that is now over the size ceiling
/// or no longer valid UTF-8 has changed since the sample; a fresh sample
/// resolves it and the caller retries. A file that is now gone follows the
/// one rule for a read after the walk (`PublishError::read_after_walk`).
/// Any other read failure (permissions, I/O error, the path becoming a
/// directory) is not a concurrent edit and retrying cannot fix it, so it
/// fails the publish with its real cause.
fn still_matches(
    file: &DiscoveredFile,
    upsert: &FileRecord,
    limits: &ResourceLimits,
) -> Result<bool, PublishError> {
    match sample_file(file, limits) {
        Ok(fresh) => Ok(fresh.identity.content_hash == upsert.identity.content_hash),
        Err(SampleError::TooLarge { .. } | SampleError::NotUtf8 { .. }) => Ok(false),
        Err(SampleError::Read { path, source }) => Err(PublishError::read_after_walk(
            &file.relative_path,
            path,
            source,
        )),
    }
}
