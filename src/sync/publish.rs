// slugaudit-line-exception: approved-by=split-judge; reason=bounded-retry CAS publish driver together with its PublishError and PublishReport contracts
//! Compare-and-swap revision publish with bounded retries (`publish`), plus
//! [`PublishError`] and [`PublishReport`].

use super::discovery;
use super::publish_attempt::try_publish;
use super::publish_log;
use super::revision::{self, RevisionError};
use super::sample;
use crate::model::ResourceLimits;
use crate::progress::ProgressSink;
use crate::util::Deadline;
use rusqlite::Connection;
use std::path::Path;
use thiserror::Error;

#[cfg(test)]
pub(super) use super::race_hook;
#[cfg(test)]
pub(super) use super::revision::{replace_unrepresentable_paths, stored_unrepresentable_paths};

/// Maximum retry attempts on CAS baseline contention or mid-sync file changes.
pub(super) const MAX_CAS_RETRIES: usize = 4;

pub(super) fn is_retryable(error: &PublishError) -> bool {
    matches!(
        error,
        PublishError::Revision(RevisionError::StaleBaseline { .. })
            | PublishError::ChangedDuringSample { .. }
    )
}

#[derive(Debug, Error)]
/// Publish-path failures. `TimeBudgetExceeded` carries the elapsed budget so
/// the tool response can name the knob rather than advise a blind retry.
pub enum PublishError {
    /// The discovery walk failed before any sampling or writing began.
    #[error(transparent)]
    Discovery(discovery::DiscoveryError),
    /// Sampling a discovered file failed.
    #[error(transparent)]
    Sample(#[from] sample::SampleError),
    /// The project's limits could not be derived from `config.json` or
    /// `SLUGAUDIT_*` environment variables.
    #[error(transparent)]
    Limits(#[from] crate::project::ProjectLimitsError),
    /// Process state read by sync was poisoned by an earlier panic.
    #[error(transparent)]
    Lock(#[from] crate::util::LockPoisoned),
    /// Writing the revision failed, including the CAS stale-baseline verdict.
    #[error(transparent)]
    Revision(#[from] revision::RevisionError),
    /// A SQLite operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// Sampled files' combined bytes exceed the per-import ceiling.
    #[error(
        "import would load {total} bytes across sampled files, exceeding the {limit}-byte ceiling"
    )]
    ImportTooLarge {
        /// Combined byte length of the sampled files.
        total: u64,
        /// Configured per-import byte ceiling.
        limit: u64,
    },
    /// Running import total overflowed `u64`.
    #[error("import total across sampled files overflowed u64, exceeding the {limit}-byte ceiling")]
    ImportTotalOverflow {
        /// Configured per-import byte ceiling.
        limit: u64,
    },
    /// A walked file changed or vanished on disk while this attempt was
    /// reading it. Retried up to `MAX_CAS_RETRIES` times.
    #[error("{path} changed on disk after being sampled; retry the call")]
    ChangedDuringSample {
        /// Project-relative path of the file that changed.
        path: String,
    },
    /// A worker could not hand a file outcome to the result drain.
    #[error("sampling result for {path} could not be delivered: {reason}")]
    SamplingResultLost {
        /// Project-relative path of the file whose outcome was lost.
        path: String,
        /// Why the result could not be delivered to the drain.
        reason: String,
    },
    /// A worker reported a file index outside the batch slice.
    #[error("sampling worker reported file index {index}, outside the {total}-file import set")]
    SamplingIndexOutOfRange {
        /// Out-of-range index reported by the worker.
        index: usize,
        /// Number of files in the import set.
        total: usize,
    },
    /// The parser pack version string is not a non-empty numeric dotted version.
    #[error("invalid parser pack version {0:?}; expected a non-empty numeric dotted version")]
    InvalidParserPackVersion(String),
    /// Wall-clock budget exhausted during `activity`.
    #[error(
        "publish exceeded its wall-clock time budget after {elapsed_ms} ms{}",
        format_budget_target(activity, path.as_deref())
    )]
    TimeBudgetExceeded {
        /// Static activity label (for example `"hashing"` or `"processing"`).
        activity: &'static str,
        /// Project-relative path of the file being processed, if any.
        path: Option<String>,
        /// Elapsed wall-clock milliseconds when the budget tripped.
        elapsed_ms: u128,
    },
}

fn format_budget_target(activity: &str, path: Option<&str>) -> String {
    match path {
        Some(path) if !path.is_empty() => format!(" while {activity} {path}"),
        _ if !activity.is_empty() => format!(" while {activity}"),
        _ => String::new(),
    }
}

impl PublishError {
    /// Elapsed wall-clock budget when this failure is a budget exhaustion.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            PublishError::TimeBudgetExceeded { elapsed_ms, .. }
            | PublishError::Revision(revision::RevisionError::TimeBudgetExceeded { elapsed_ms }) => {
                Some(*elapsed_ms)
            }
            PublishError::Discovery(error) => error.budget_elapsed_ms(),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn is_corruption(&self) -> bool {
        match self {
            PublishError::Database(err) => crate::store::is_rusqlite_corruption(err),
            PublishError::Revision(revision::RevisionError::Database(err)) => {
                crate::store::is_rusqlite_corruption(err)
            }
            _ => false,
        }
    }

    /// Constructs a [`PublishError::TimeBudgetExceeded`] naming `activity`,
    /// `relative_path`, and elapsed wall-clock milliseconds.
    pub(super) fn time_budget_exceeded(
        activity: &'static str,
        relative_path: &str,
        elapsed: std::time::Duration,
    ) -> Self {
        Self::TimeBudgetExceeded {
            activity,
            path: Some(relative_path.to_owned()),
            elapsed_ms: elapsed.as_millis(),
        }
    }

    /// Maps a post-walk file read error: `NotFound` becomes retryable
    /// [`PublishError::ChangedDuringSample`], any other I/O error fails closed.
    pub(super) fn read_after_walk(
        relative_path: &str,
        path: std::path::PathBuf,
        source: std::io::Error,
    ) -> PublishError {
        if discovery::is_vanished(&source) {
            PublishError::ChangedDuringSample {
                path: relative_path.to_owned(),
            }
        } else {
            PublishError::Sample(sample::SampleError::Read { path, source })
        }
    }
}

impl From<discovery::DiscoveryError> for PublishError {
    fn from(error: discovery::DiscoveryError) -> Self {
        match error {
            discovery::DiscoveryError::Vanished { relative_path } => {
                PublishError::ChangedDuringSample {
                    path: relative_path,
                }
            }
            other => PublishError::Discovery(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Summary of one committed (or unchanged) publish operation.
pub struct PublishReport {
    /// The `rev-N` id of the current revision after this publish.
    pub revision_id: String,
    /// Files newly added to the project's discovered existence set.
    pub added: usize,
    /// Files whose content hash or classification changed since the previous revision.
    pub modified: usize,
    /// Files present in the previous revision that are no longer on disk.
    pub deleted: usize,
    /// Files whose content hash matches the previous revision.
    pub unchanged: usize,
    /// Files discovery or sampling skipped individually, with reasons.
    pub skipped: Vec<discovery::SkippedFile>,
}

/// Discovers, hashes, and diffs the project's current disk state against
/// stored rows, then publishes the delta as one atomic CAS revision.
///
/// # Errors
///
/// Returns [`PublishError`] if discovery, sampling, database operations, or
/// bounded CAS retries fail.
pub fn publish(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
    sink: &dyn ProgressSink,
) -> Result<PublishReport, PublishError> {
    let limits = crate::project::effective_limits(root)?;
    publish_with_limits(connection, root, parser_pack_version, sink, &limits)
}

/// [`publish`] under explicit [`ResourceLimits`], sharing one wall-clock
/// [`Deadline`] across every CAS retry.
pub(crate) fn publish_with_limits(
    connection: &mut Connection,
    root: &Path,
    parser_pack_version: &str,
    sink: &dyn ProgressSink,
    limits: &ResourceLimits,
) -> Result<PublishReport, PublishError> {
    if !is_valid_parser_pack_version(parser_pack_version) {
        return Err(PublishError::InvalidParserPackVersion(
            parser_pack_version.to_owned(),
        ));
    }
    sink.emit(crate::progress::ProgressEvent::Started {
        phase: "publishing",
    });
    let deadline = Deadline::after(limits.max_sync_wall_clock);
    let mut attempt = 0_usize;
    let result = loop {
        let result = try_publish(
            connection,
            root,
            parser_pack_version,
            sink,
            limits,
            &deadline,
        );
        if let Err(error) = &result
            && is_retryable(error)
            && attempt < MAX_CAS_RETRIES
            && deadline.exceeded().is_none()
            && let Some(next_attempt) = attempt.checked_add(1)
        {
            let (error_kind, path) = publish_log::failure_fields(error);
            if let Some(path) = path {
                tracing::info!(
                    attempt,
                    error_kind,
                    path = %path.display(),
                    "publish retrying"
                );
            } else {
                tracing::info!(attempt, error_kind, "publish retrying");
            }
            attempt = next_attempt;
            continue;
        }
        if let Ok(report) = &result {
            publish_log::log_completed(report, attempt);
        }
        break result;
    };
    sink.emit(crate::progress::ProgressEvent::Completed {
        phase: "publishing",
    });
    result
}

fn is_valid_parser_pack_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "publish_race_tests.rs"]
mod race_tests;

#[cfg(test)]
#[path = "publish_edges_tests.rs"]
mod edges_tests;

#[cfg(test)]
#[path = "publish_acceptance_tests.rs"]
mod acceptance_tests;

#[cfg(test)]
#[path = "publish_mutation_tests.rs"]
mod mutation_tests;
