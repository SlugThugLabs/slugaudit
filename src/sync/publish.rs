use super::discovery;
use super::revision;
use super::sample;
use thiserror::Error;

pub use super::publish_cas::publish;

// Re-exports for the test modules, which are children of this module and
// access these items via `super::*`. The definitions live in their natural
// homes; this module just makes them reachable from tests. Gated behind
// `cfg(test)` because nothing in the library build uses them directly.
#[cfg(test)]
pub(super) use super::publish_cas::{MAX_CAS_RETRIES, is_retryable, publish_with_limits};
#[cfg(test)]
pub(super) use super::publish_diff::build_upserts_and_deletions;
#[cfg(test)]
pub(super) use super::race_hook;
#[cfg(test)]
pub(super) use super::revalidate::revalidate_unchanged_since_sample;
#[cfg(test)]
pub(super) use super::revision::RevisionError;
#[cfg(test)]
pub(super) use super::sample::sample_all_with_deadline;
#[cfg(test)]
pub(super) use crate::model::ResourceLimits;
#[cfg(test)]
pub(super) use rusqlite::Connection;

#[derive(Debug, Error)]
/// Publish-path failures. `TimeBudgetExceeded` carries the elapsed budget so
/// the tool response can name the knob rather than advise a blind retry.
pub enum PublishError {
    #[error(transparent)]
    Discovery(#[from] discovery::DiscoveryError),
    #[error(transparent)]
    Sample(#[from] sample::SampleError),
    #[error(transparent)]
    Revision(#[from] revision::RevisionError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "import would load {total} bytes across sampled files, exceeding the {limit}-byte ceiling"
    )]
    ImportTooLarge { total: u64, limit: u64 },
    /// A file's on-disk content changed between being sampled and the
    /// publish transaction that would have written it as current. Caught by
    /// `revalidate_unchanged_since_sample` immediately before the write;
    /// the caller retries with an entirely fresh sample rather than
    /// publishing a revision built from stale bytes. When this error
    /// reaches the caller (after CAS retries are exhausted), the caller
    /// should retry the whole tool call.
    #[error("{path} changed on disk after being sampled; retry the call")]
    ChangedDuringSample { path: String },
    /// A worker could not hand a sampled file to the result drain (the
    /// receiving end was gone). The file's outcome would have been lost, so
    /// the publish fails instead of omitting it.
    #[error("sampling result for {path} could not be delivered: {reason}")]
    SamplingResultLost { path: String, reason: String },
    /// A sampling worker reported an index outside the discovered file set.
    /// Workers only claim indices below the set's length, so reaching this
    /// means an internal invariant was violated; the publish fails rather
    /// than indexing a file that does not exist or silently dropping a
    /// result.
    #[error("sampling worker reported file index {index}, outside the {total}-file import set")]
    SamplingIndexOutOfRange { index: usize, total: usize },
    #[error("invalid parser pack version {0:?}; expected a non-empty numeric dotted version")]
    InvalidParserPackVersion(String),
    /// `path` is a pre-formatted note naming the file being processed when
    /// the budget tripped (e.g. `" while processing src/gen/big.rs"`), or
    /// empty when the tripping site has no single file in hand (e.g. the
    /// diff phase). Kept pre-formatted so the error reads cleanly without
    /// a custom Display impl.
    #[error("publish exceeded its wall-clock time budget after {elapsed_ms} ms{path}")]
    TimeBudgetExceeded { path: String, elapsed_ms: u128 },
}

impl PublishError {
    /// The elapsed wall-clock budget when this failure is the budget being
    /// exhausted — including one raised by discovery *inside* a publish,
    /// which arrives wrapped — else `None`. The map site pairs this with
    /// `crate::error_codes::budget_payload` so a budget failure carries
    /// `sync_budget_exhausted` and its knob, rather than reading as a
    /// transient failure the caller should retry.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            PublishError::TimeBudgetExceeded { elapsed_ms, .. } => Some(*elapsed_ms),
            PublishError::Discovery(error) => error.budget_elapsed_ms(),
            _ => None,
        }
    }

    /// Static category for a log line about this failure. Never the error's
    /// `Display`: several variants embed a path or file content in it.
    #[must_use]
    pub(crate) const fn category(&self) -> &'static str {
        match self {
            PublishError::Discovery(_) => "discovery",
            PublishError::Sample(_) => "sample",
            PublishError::Revision(_) => "revision",
            PublishError::Database(_) => "database",
            PublishError::ImportTooLarge { .. } => "import_too_large",
            PublishError::ChangedDuringSample { .. } => "changed_during_sample",
            PublishError::SamplingResultLost { .. } => "sampling_result_lost",
            PublishError::SamplingIndexOutOfRange { .. } => "sampling_index_out_of_range",
            PublishError::InvalidParserPackVersion(_) => "invalid_parser_pack_version",
            PublishError::TimeBudgetExceeded { .. } => "time_budget_exceeded",
        }
    }

    /// Logs that this failure lost the parallel sampling pool's first-error
    /// race. Only the static category and a redacted path id are emitted;
    /// the raw `Display` of several variants embeds a path.
    pub(super) fn warn_sampling_superseded(&self) {
        tracing::warn!(
            operation = "sample_batch",
            category = self.category(),
            path_id = ?self.redacted_path_id(),
            "a later sampling failure was discarded after the first error was recorded",
        );
    }

    /// A stable, non-reversible id for the path this failure carries, if it
    /// carries one — the only form of it that may enter a log line.
    /// [`PublishError::TimeBudgetExceeded::path`] is a pre-formatted note
    /// (which can embed a relative path), not a path, so it is not exposed
    /// here.
    #[must_use]
    pub(crate) fn redacted_path_id(&self) -> Option<String> {
        match self {
            PublishError::ChangedDuringSample { path }
            | PublishError::SamplingResultLost { path, .. }
            | PublishError::Revision(revision::RevisionError::InvalidParserRun { path, .. }) => {
                Some(crate::util::redacted_log_id(path))
            }
            PublishError::Sample(
                sample::SampleError::Read { path, .. } | sample::SampleError::TooLarge { path, .. },
            )
            | PublishError::Discovery(
                discovery::DiscoveryError::Read { path, .. }
                | discovery::DiscoveryError::NotRelative(path)
                | discovery::DiscoveryError::NonUtf8Path(path),
            ) => Some(crate::util::redacted_path(path)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// What one publish attempt changed — added/modified/deleted/unchanged counts
/// and the per-file skips, ordered deterministically.
pub struct PublishReport {
    pub revision_id: String,
    /// Counts describe the project's *file set* — every discovered file,
    /// including ones only an existence row describes (`skipped`). A skipped
    /// file new to the project counts as added; it is listed in `skipped`
    /// with why. `added + modified + unchanged` is therefore the size of the
    /// current existence set, not the number of indexed files.
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub unchanged: usize,
    /// Files the discovery walk found but could not include (unreadable,
    /// non-UTF8 path, oversize, etc.), with why. Never fails the publish by
    /// itself. Those whose path is representable got a `files` row with
    /// `file_kind = 'skipped'`; a non-UTF-8 name cannot be stored and is
    /// only visible here and in the log.
    pub skipped: Vec<discovery::SkippedFile>,
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
