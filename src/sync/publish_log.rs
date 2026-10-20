//! Structured tracing for one `publish()` attempt's terminal outcome —
//! split out to keep `publish.rs`'s own retry-loop control flow readable.
use super::discovery::DiscoveryError;
use super::publish::{PublishError, PublishReport};
use super::revision::RevisionError;
use super::sample::SampleError;
use std::path::Path;

/// Logs the terminal outcome of one `publish()` attempt. This is the single
/// warn site for a failed publish: every caller (the manager paths and
/// `project_control::enable`) reaches it, so one failure is never logged
/// twice.
pub(super) fn log_outcome(
    root: &Path,
    result: &Result<PublishReport, PublishError>,
    retries: usize,
) {
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
        Err(error) => {
            let (error_kind, path) = failure_fields(error);
            match path {
                Some(path) => tracing::warn!(
                    root_id = %crate::util::redacted_path(root),
                    operation = "publish",
                    retries,
                    error_kind,
                    path_id = %crate::util::redacted_path(path),
                    "publish failed"
                ),
                None => tracing::warn!(
                    root_id = %crate::util::redacted_path(root),
                    operation = "publish",
                    retries,
                    error_kind,
                    "publish failed"
                ),
            }
        }
    }
}

/// The static failure category for a publish error, plus the path the
/// variant carries when it has one.
///
/// Log sites correlate a failure through these fields instead of the
/// error's `Display`: `SampleError::Read`/`TooLarge` and
/// `DiscoveryError::Read` embed an absolute path in their text, and
/// `ChangedDuringSample`/`InvalidParserRun`/`SamplingResultLost` embed a
/// project-relative one, all of which the path-logging rule redacts.
pub(super) fn failure_fields(error: &PublishError) -> (&'static str, Option<&Path>) {
    match error {
        PublishError::Discovery(error) => match error {
            DiscoveryError::Walk(_) => ("discovery_walk", None),
            DiscoveryError::Read { path, .. } => ("discovery_read", Some(path)),
            DiscoveryError::NotRelative(path) => ("discovery_not_relative", Some(path)),
            DiscoveryError::NonUtf8Path(path) => ("discovery_non_utf8_path", Some(path)),
            DiscoveryError::TimeBudgetExceeded { .. } => ("discovery_time_budget_exceeded", None),
        },
        PublishError::Sample(SampleError::Read { path, .. }) => ("sample_read", Some(path)),
        PublishError::Sample(SampleError::TooLarge { path, .. }) => {
            ("sample_too_large", Some(path))
        }
        PublishError::Revision(RevisionError::Database(_)) => ("revision_database", None),
        PublishError::Revision(RevisionError::InvalidParserRun { path, .. }) => {
            ("revision_invalid_parser_run", Some(Path::new(path)))
        }
        PublishError::Revision(RevisionError::StaleBaseline { .. }) => {
            ("revision_stale_baseline", None)
        }
        PublishError::Database(_) => ("database", None),
        PublishError::ImportTooLarge { .. } => ("import_too_large", None),
        PublishError::ChangedDuringSample { path } => {
            ("changed_during_sample", Some(Path::new(path)))
        }
        PublishError::SamplingResultLost { path, .. } => {
            ("sampling_result_lost", Some(Path::new(path)))
        }
        PublishError::InvalidParserPackVersion(_) => ("invalid_parser_pack_version", None),
        PublishError::TimeBudgetExceeded { .. } => ("time_budget_exceeded", None),
    }
}
