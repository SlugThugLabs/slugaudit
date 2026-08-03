//! Structured tracing for one `publish()` attempt's terminal outcome —
//! split out to keep `publish.rs`'s own retry-loop control flow readable.
use super::publish::{PublishError, PublishReport};

pub(super) fn log_outcome(result: &Result<PublishReport, PublishError>, retries: usize) {
    match result {
        Ok(report) => tracing::info!(
            revision_id = report.revision_id,
            added = report.added,
            modified = report.modified,
            deleted = report.deleted,
            unchanged = report.unchanged,
            retries,
            "publish completed"
        ),
        Err(error) => tracing::warn!(retries, error = %error, "publish failed"),
    }
}
