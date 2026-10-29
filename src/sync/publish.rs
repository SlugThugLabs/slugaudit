use super::discovery;
use super::revision;
use super::sample;
use std::collections::HashSet;
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
    /// The discovery walk failed before any sampling or writing began.
    #[error(transparent)]
    Discovery(#[from] discovery::DiscoveryError),
    /// Sampling a discovered file failed.
    #[error(transparent)]
    Sample(#[from] sample::SampleError),
    /// The project's limits could not be derived: its `config.json` is
    /// present but unreadable or malformed.
    #[error(transparent)]
    Limits(#[from] crate::project::ProjectLimitsError),
    /// Writing the revision failed, including the CAS stale-baseline verdict.
    #[error(transparent)]
    Revision(#[from] revision::RevisionError),
    /// A SQLite operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// The sampled files' combined bytes exceed the import ceiling, so the
    /// publish fails before loading them.
    #[error(
        "import would load {total} bytes across sampled files, exceeding the {limit}-byte ceiling"
    )]
    ImportTooLarge {
        /// Combined byte length of the sampled files.
        total: u64,
        /// The configured per-import byte ceiling.
        limit: u64,
    },
    /// A file's on-disk content changed between being sampled and the
    /// publish transaction that would have written it as current. Caught by
    /// `revalidate_unchanged_since_sample` immediately before the write;
    /// the caller retries with an entirely fresh sample rather than
    /// publishing a revision built from stale bytes. When this error
    /// reaches the caller (after CAS retries are exhausted), the caller
    /// should retry the whole tool call.
    #[error("{path} changed on disk after being sampled; retry the call")]
    ChangedDuringSample {
        /// Project-relative path of the file that changed.
        path: String,
    },
    /// A worker could not hand a sampled file to the result drain (the
    /// receiving end was gone). The file's outcome would have been lost, so
    /// the publish fails instead of omitting it.
    #[error("sampling result for {path} could not be delivered: {reason}")]
    SamplingResultLost {
        /// Project-relative path of the file whose outcome was lost.
        path: String,
        /// Why the result could not be delivered to the drain.
        reason: String,
    },
    /// A sampling worker reported an index outside the discovered file set.
    /// Workers only claim indices below the set's length, so reaching this
    /// means an internal invariant was violated; the publish fails rather
    /// than indexing a file that does not exist or silently dropping a
    /// result.
    #[error("sampling worker reported file index {index}, outside the {total}-file import set")]
    SamplingIndexOutOfRange {
        /// The out-of-range index the worker reported.
        index: usize,
        /// Number of files in the import set.
        total: usize,
    },
    /// The build's parser pack version is not a non-empty numeric dotted
    /// version, so a revision cannot be pinned against it.
    #[error("invalid parser pack version {0:?}; expected a non-empty numeric dotted version")]
    InvalidParserPackVersion(String),
    /// `path` is a pre-formatted note naming the file being processed when
    /// the budget tripped (e.g. `" while processing src/gen/big.rs"`), or
    /// empty when the tripping site has no single file in hand (e.g. the
    /// diff phase). Kept pre-formatted so the error reads cleanly without
    /// a custom Display impl.
    #[error("publish exceeded its wall-clock time budget after {elapsed_ms} ms{path}")]
    TimeBudgetExceeded {
        /// Pre-formatted note naming the file being processed, or empty.
        path: String,
        /// Elapsed wall-clock milliseconds when the budget tripped.
        elapsed_ms: u128,
    },
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
            PublishError::Limits(_) => "project_limits",
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
    /// race. The category, the real path (when the variant names one) and
    /// the error's `Display` are emitted.
    pub(super) fn warn_sampling_superseded(&self) {
        tracing::warn!(
            operation = "sample_batch",
            category = self.category(),
            path = ?self.path_for_log(),
            error = %self,
            "a later sampling failure was discarded after the first error was recorded",
        );
    }

    /// The path this failure carries, if it carries one, for log lines.
    /// [`PublishError::TimeBudgetExceeded::path`] is a pre-formatted note,
    /// not a path, so it is not exposed here.
    fn path_for_log(&self) -> Option<String> {
        match self {
            PublishError::ChangedDuringSample { path }
            | PublishError::SamplingResultLost { path, .. }
            | PublishError::Revision(revision::RevisionError::InvalidParserRun { path, .. }) => {
                Some(path.clone())
            }
            PublishError::Sample(
                sample::SampleError::Read { path, .. } | sample::SampleError::TooLarge { path, .. },
            )
            | PublishError::Discovery(
                discovery::DiscoveryError::Read { path, .. }
                | discovery::DiscoveryError::NotRelative(path)
                | discovery::DiscoveryError::NonUtf8Path(path),
            ) => Some(path.display().to_string()),
            PublishError::Discovery(discovery::DiscoveryError::Walk(error)) => {
                crate::watch::errors::ignore_error_path(error)
                    .map(|path| path.display().to_string())
            }
            PublishError::Discovery(discovery::DiscoveryError::IgnoreRules {
                path: Some(path),
                ..
            }) => Some(path.display().to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// What one publish attempt changed — added/modified/deleted/unchanged counts
/// and the per-file skips, ordered deterministically.
pub struct PublishReport {
    /// The `rev-N` id of the revision this publish committed.
    pub revision_id: String,
    /// Counts describe the project's *file set* — every discovered file,
    /// including ones only an existence row describes (`skipped`). A skipped
    /// file new to the project counts as added; it is listed in `skipped`
    /// with why. `added + modified + unchanged` is therefore the size of the
    /// current existence set, not the number of indexed files.
    pub added: usize,
    /// Files whose content hash differs from the previous revision.
    pub modified: usize,
    /// Files indexed by the previous revision but no longer on disk.
    pub deleted: usize,
    /// Files whose content hash matches the previous revision.
    pub unchanged: usize,
    /// Files the discovery walk found but could not include (unreadable,
    /// non-UTF8 path, oversize, etc.), with why. Never fails the publish by
    /// itself. Those whose path is representable got a `files` row with
    /// `file_kind = 'skipped'`; a non-UTF-8 name cannot be stored and is
    /// only visible here and in the log.
    pub skipped: Vec<discovery::SkippedFile>,
}

/// Replaces the persisted set of discovered non-UTF-8 paths with the set a
/// walk just produced. The table is disposable derived state: every full
/// discovery and every incremental stat sweep walks the whole tree, so the
/// replacement is exact, and an unchanged set writes nothing. A failed write
/// fails the sync pass — `report` must never read a count its latest walk
/// did not produce.
pub(super) fn replace_unrepresentable_paths(
    connection: &mut rusqlite::Connection,
    paths: &HashSet<Vec<u8>>,
) -> Result<(), rusqlite::Error> {
    if stored_unrepresentable_paths(connection)? == *paths {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM unrepresentable_paths", [])?;
    {
        let mut insert =
            tx.prepare("INSERT OR IGNORE INTO unrepresentable_paths (path) VALUES (?1)")?;
        for path in paths {
            insert.execute([path])?;
        }
    }
    tx.commit()
}

fn stored_unrepresentable_paths(
    connection: &rusqlite::Connection,
) -> Result<HashSet<Vec<u8>>, rusqlite::Error> {
    let mut statement = connection.prepare("SELECT path FROM unrepresentable_paths")?;
    let paths = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<Result<_, _>>()?;
    Ok(paths)
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
