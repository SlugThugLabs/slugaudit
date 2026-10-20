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
        PublishError::Discovery(error) => discovery_failure_fields(error),
        PublishError::Sample(error) => sample_failure_fields(error),
        PublishError::Revision(error) => revision_failure_fields(error),
        PublishError::Database(_) => ("database", None),
        PublishError::ImportTooLarge { .. } => ("import_too_large", None),
        PublishError::ChangedDuringSample { path } => {
            ("changed_during_sample", Some(Path::new(path)))
        }
        PublishError::SamplingResultLost { path, .. } => {
            ("sampling_result_lost", Some(Path::new(path)))
        }
        PublishError::SamplingIndexOutOfRange { .. } => ("sampling_index_out_of_range", None),
        PublishError::InvalidParserPackVersion(_) => ("invalid_parser_pack_version", None),
        PublishError::TimeBudgetExceeded { .. } => ("time_budget_exceeded", None),
    }
}

fn discovery_failure_fields(error: &DiscoveryError) -> (&'static str, Option<&Path>) {
    match error {
        DiscoveryError::Walk(error) => (
            "discovery_walk",
            crate::watch::errors::ignore_error_path(error),
        ),
        DiscoveryError::IgnoreRules { path, .. } => ("discovery_ignore_rules", path.as_deref()),
        DiscoveryError::Read { path, .. } => ("discovery_read", Some(path)),
        DiscoveryError::NotRelative(path) => ("discovery_not_relative", Some(path)),
        DiscoveryError::NonUtf8Path(path) => ("discovery_non_utf8_path", Some(path)),
        DiscoveryError::TimeBudgetExceeded { .. } => ("discovery_time_budget_exceeded", None),
    }
}

fn sample_failure_fields(error: &SampleError) -> (&'static str, Option<&Path>) {
    match error {
        SampleError::Read { path, .. } => ("sample_read", Some(path)),
        SampleError::TooLarge { path, .. } => ("sample_too_large", Some(path)),
    }
}

fn revision_failure_fields(error: &RevisionError) -> (&'static str, Option<&Path>) {
    match error {
        RevisionError::Database(_) => ("revision_database", None),
        RevisionError::InvalidParserRun { path, .. } => {
            ("revision_invalid_parser_run", Some(Path::new(path)))
        }
        RevisionError::StaleBaseline { .. } => ("revision_stale_baseline", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sampling_index_error_maps_to_its_static_category_without_a_path() {
        let error = PublishError::SamplingIndexOutOfRange { index: 7, total: 3 };

        let (error_kind, path) = failure_fields(&error);

        assert_eq!(error_kind, "sampling_index_out_of_range");
        assert_eq!(path, None);
    }
}
