//! Compare-and-swap publish with retry. Wraps a single publish attempt
//! (`publish_attempt::try_publish`) in a retry loop that handles the two
//! retryable hazards: another publisher winning the CAS race on the current
//! revision id, and a file changing on disk after being sampled but before
//! the write transaction that would record it.

use super::publish::{PublishError, PublishReport};
use super::publish_attempt::try_publish;
use super::publish_log;
use super::revision::RevisionError;
use rusqlite::Connection;
use std::path::Path;

/// How many times a retryable publish failure is retried before the error
/// surfaces to the tool caller. Each retry re-discovers and re-samples the
/// filesystem from scratch — it never reuses a stale sample.
pub(super) const MAX_CAS_RETRIES: usize = 4;

pub(super) fn is_retryable(error: &PublishError) -> bool {
    matches!(
        error,
        PublishError::Revision(RevisionError::StaleBaseline { .. })
            | PublishError::ChangedDuringSample { .. }
    )
}

/// Discovers, hashes, and diffs the project's current disk state against
/// what the database has stored, then publishes exactly the delta as one
/// atomic CAS revision. A disk state identical to the current revision
/// skips publishing entirely rather than churning out a no-op revision.
///
/// Two independent hazards are retried the same way, up to
/// `MAX_CAS_RETRIES` times, each retry re-sampling from scratch: another
/// publisher committing first (compare-and-swap on the current revision
/// id), and a file changing on disk after this attempt sampled it but
/// before the write transaction it would land in. Neither is silently
/// papered over — both fail closed into a retry rather than publishing a
/// revision that doesn't match a real, stable disk state.
///
/// # Errors
///
/// Returns an error if discovery, sampling, or any database operation in
/// the publish transaction fails (including exhausting retries).
pub fn publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
) -> Result<PublishReport, PublishError> {
    let mut attempt = 0_usize;
    loop {
        let result = try_publish(connection, root, parser_pack_version);
        if let Err(error) = &result
            && is_retryable(error)
            && attempt + 1 < MAX_CAS_RETRIES
        {
            tracing::info!(attempt, reason = %error, "publish retrying");
            attempt += 1;
            continue;
        }
        publish_log::log_outcome(&result, attempt);
        return result;
    }
}
