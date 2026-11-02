//! Structured tracing for a completed `publish()`, and the fields that
//! describe a failed one — kept apart so `publish.rs`'s own retry-loop
//! control flow stays readable.
use super::discovery::DiscoveryError;
use super::publish::{PublishError, PublishReport};
use super::revision::RevisionError;
use super::sample::SampleError;
use std::path::Path;

/// Logs a completed `publish()`. A failed publish is not logged here: it is
/// returned, and each caller either returns it to the tool call with
/// [`failure_context`] in the message (logged once by
/// `server_runner::log_completion`) or recovers from it and logs the recovery
/// (`recovery_log`).
pub(super) fn log_completed(report: &PublishReport, retries: usize) {
    tracing::info!(
        revision_id = report.revision_id,
        added = report.added,
        modified = report.modified,
        deleted = report.deleted,
        unchanged = report.unchanged,
        retries,
        "publish completed"
    );
}

/// The context a failed publish's tool error carries beyond the error's own
/// `Display`: the project root, the static failure category and, when the
/// variant names one, the file. `server_runner::log_completion` logs the
/// tool error's message, so this is what makes the failure diagnosable from
/// that one log line.
pub(crate) fn failure_context(root: &Path, error: &PublishError) -> String {
    let (error_kind, path) = failure_fields(error);
    match path {
        Some(path) => format!(
            "root: {}; error_kind: {error_kind}; path: {}",
            root.display(),
            path.display()
        ),
        None => format!("root: {}; error_kind: {error_kind}", root.display()),
    }
}

/// The static failure category for a publish error, plus the path the
/// variant carries when it has one.
///
/// The single mapping from a publish failure to its log fields: every site
/// that names a publish failure (a retry, a superseded or unrecorded worker
/// failure, the returned tool error's [`failure_context`]) uses these
/// alongside the error's `Display`, so a failure can be filtered by category
/// and by the file it names.
pub(super) fn failure_fields(error: &PublishError) -> (&'static str, Option<&Path>) {
    match error {
        PublishError::Discovery(error) => discovery_failure_fields(error),
        PublishError::Sample(error) => sample_failure_fields(error),
        PublishError::Revision(error) => revision_failure_fields(error),
        PublishError::Limits(_) => ("project_limits", None),
        PublishError::Lock(_) => ("lock_poisoned", None),
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
            crate::ignore_rules::ignore_error_path(error),
        ),
        DiscoveryError::IgnoreRules { path, .. } => ("discovery_ignore_rules", path.as_deref()),
        DiscoveryError::Read { path, .. } => ("discovery_read", Some(path)),
        DiscoveryError::NotRelative(path) => ("discovery_not_relative", Some(path)),
        DiscoveryError::NonUtf8Path(path) => ("discovery_non_utf8_path", Some(path)),
        DiscoveryError::Vanished { relative_path } => {
            ("discovery_vanished", Some(Path::new(relative_path)))
        }
        DiscoveryError::TimeBudgetExceeded { .. } => ("discovery_time_budget_exceeded", None),
    }
}

fn sample_failure_fields(error: &SampleError) -> (&'static str, Option<&Path>) {
    match error {
        SampleError::Read { path, .. } => ("sample_read", Some(path)),
        SampleError::TooLarge { path, .. } => ("sample_too_large", Some(path)),
        SampleError::NotUtf8 { path } => ("sample_not_utf8", Some(path)),
    }
}

fn revision_failure_fields(error: &RevisionError) -> (&'static str, Option<&Path>) {
    match error {
        RevisionError::Database(_) => ("revision_database", None),
        RevisionError::InvalidParserRun { path, .. } => {
            ("revision_invalid_parser_run", Some(Path::new(path)))
        }
        RevisionError::StaleBaseline { .. } => ("revision_stale_baseline", None),
        RevisionError::Clock(_) => ("revision_clock", None),
    }
}

#[cfg(test)]
#[path = "publish_log_tests.rs"]
mod tests;
