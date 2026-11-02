//! Pass 2 of a full sync: sample the changed files on the shared worker
//! pool ([`super::file_pool`]).
//!
//! `sample_all_with_deadline` feeds `super::sample::to_file_record`. It is
//! kept apart from `sample.rs`, which owns sampling one file
//! (read/hash/classify, evidence capping); this module owns the pass over
//! many files: per-file skips, the import ceiling and progress events.
//! `publish_attempt` calls it directly.
//!
//! Files that individually exceed `limits.max_file_bytes` are **skipped
//! per-file** rather than failing the publish: such a file cannot be
//! indexed at all, and letting one oversized generated bundle make the
//! whole project unqueryable is worse than reporting it. The skip reason
//! is returned (sorted by path for deterministic report ordering) so the
//! caller can surface it in the publish report, mirroring how discovery
//! skips unreadable files. A file that is not valid UTF-8 is skipped the
//! same way. Only the project-wide total-byte ceiling and hard errors
//! remain fatal.

use super::discovery::{DiscoveredFile, SkippedFile};
use super::file_pool::{self, Pass};
use super::publish::PublishError;
use super::revision_skipped::{SKIP_REASON_NOT_UTF8, SKIP_REASON_OVERSIZE};
use super::sample::{Sample, SampleError, sample_file};
use crate::model::ResourceLimits;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::util::Deadline;
use std::sync::atomic::{AtomicU64, Ordering};

/// The sampling pass, as its log lines and time-budget error name it.
const SAMPLE_PASS: Pass = Pass {
    operation: "sample_batch",
    activity: "processing",
};

/// One file's Pass 2 outcome.
enum Sampled {
    /// The file's content and identity.
    Sample(Sample),
    /// The file cannot be indexed and becomes a skipped row.
    Skipped(SkippedFile),
}

/// Samples every discovered file, accumulating the total byte count and
/// failing closed if the project-wide import ceiling is exceeded — or if
/// an explicit [`Deadline`] (checked once per file) is spent, so a
/// pathological repo (a huge tree, a hung filesystem) fails closed with
/// [`PublishError::TimeBudgetExceeded`] instead of stalling the publish
/// forever. The deadline is created by the caller so the whole operation
/// shares one budget across sampling, diffing, revalidation, and retries.
/// Per-file oversize and non-UTF-8 skips are documented at the module level.
///
/// Runs on the shared bounded worker pool ([`file_pool::run`]): a 60 s sync
/// deadline compounded with fully serialized sampling would make a 60 k-file
/// first import the single most likely production failure mode. The pool
/// returns samples in discovered order, so downstream consumers
/// (`manifest::compare`, `aggregate_manifest_hash`,
/// `build_upserts_and_deletions`) see the discovered file order. Per-file
/// `Sampling` events arrive at the sink in worker-completion order rather
/// than file-index order; the receiving sink (`McpProgressSink` /
/// `NoopProgressSink`) already tolerates out-of-order updates because file
/// sampling is intrinsically parallel and the `i/N` ratio is what consumers
/// display.
///
/// The byte-cap check uses `fetch_update` so the limit is enforced across
/// concurrent workers: a worker that would push the total above
/// `max_total_import_bytes` fails the run with
/// [`PublishError::ImportTooLarge`].
pub(super) fn sample_all_with_deadline(
    discovered: &[DiscoveredFile],
    limits: &ResourceLimits,
    sink: &dyn ProgressSink,
    deadline: &Deadline,
) -> Result<(Vec<Sample>, Vec<SkippedFile>), PublishError> {
    let total = discovered.len();
    let total_bytes = AtomicU64::new(0);
    let outcomes = file_pool::run(discovered, deadline, SAMPLE_PASS, |idx, file| {
        let outcome = sample_one(file, limits, &total_bytes)?;
        sink.emit(ProgressEvent::Sampling {
            phase: "publishing",
            current: idx + 1,
            total,
        });
        Ok(outcome)
    })?;
    let mut samples = Vec::with_capacity(outcomes.len());
    let mut skipped = Vec::new();
    for outcome in outcomes {
        match outcome {
            Sampled::Sample(sample) => samples.push(sample),
            Sampled::Skipped(skip) => skipped.push(skip),
        }
    }
    // Deterministic report ordering, mirroring `discovery`'s sort of its own
    // skipped files.
    skipped.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
    Ok((samples, skipped))
}

/// Samples one file and claims its bytes against the import ceiling. A file
/// over the per-file ceiling or not valid UTF-8 cannot be indexed at all: it
/// is skipped like discovery skips an unreadable file, and every other file
/// keeps going.
///
/// # Errors
///
/// [`PublishError::ImportTooLarge`] when the file's bytes would push the
/// run past the import ceiling, [`PublishError::ChangedDuringSample`] for a
/// file deleted since the walk, and the read error for any other read
/// failure.
fn sample_one(
    file: &DiscoveredFile,
    limits: &ResourceLimits,
    total_bytes: &AtomicU64,
) -> Result<Sampled, PublishError> {
    let (error, code) = match sample_file(file, limits) {
        Ok(sample) => {
            claim_bytes(total_bytes, sample.byte_len, limits.max_total_import_bytes)?;
            return Ok(Sampled::Sample(sample));
        }
        Err(SampleError::Read { path, source }) => {
            return Err(PublishError::read_after_walk(
                &file.relative_path,
                path,
                source,
            ));
        }
        Err(error @ SampleError::TooLarge { .. }) => (error, SKIP_REASON_OVERSIZE),
        Err(error @ SampleError::NotUtf8 { .. }) => (error, SKIP_REASON_NOT_UTF8),
    };
    Ok(Sampled::Skipped(SkippedFile {
        absolute_path: file.absolute_path.clone(),
        reason: error.to_string(),
        code,
    }))
}

/// Adds `byte_len` to the running import total. `fetch_update` is the only
/// correct primitive: a plain load/store would race across workers and let
/// two threads each push past the cap before either notices.
///
/// # Errors
///
/// [`PublishError::ImportTooLarge`] when the add would exceed `limit`.
fn claim_bytes(total_bytes: &AtomicU64, byte_len: u64, limit: u64) -> Result<(), PublishError> {
    match total_bytes.fetch_update(Ordering::AcqRel, Ordering::Acquire, |prev| {
        let next = prev.saturating_add(byte_len);
        (next <= limit).then_some(next)
    }) {
        Ok(_) => Ok(()),
        // `Err` carries the total this claim was refused against.
        Err(total_before) => Err(PublishError::ImportTooLarge {
            total: total_before.saturating_add(byte_len),
            limit,
        }),
    }
}

#[cfg(test)]
#[path = "sample_batch_tests.rs"]
mod tests;
