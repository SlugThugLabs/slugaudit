//! Re-validates that files haven't changed on disk between sampling and the
//! publish transaction that would record them. A file whose content changed
//! in that window must not be published as if it were stable — the caller
//! retries with a fresh sample instead.

use super::discovery::DiscoveredFile;
use super::publish::PublishError;
use super::revision::FileRecord;
use super::sample::sample_file;
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
            return Err(PublishError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let unchanged = by_path
            .get(upsert.relative_path.as_str())
            .is_some_and(|file| {
                sample_file(file, limits)
                    .is_ok_and(|fresh| fresh.identity.content_hash == upsert.identity.content_hash)
            });
        if !unchanged {
            return Err(PublishError::ChangedDuringSample {
                path: upsert.relative_path.clone(),
            });
        }
    }
    Ok(())
}
