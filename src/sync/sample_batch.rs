//! Pass 2 of a full sync: sample and parse changed files in bounded 256-file
//! chunks on a persistent scoped worker pool ([`super::file_pool::with_pool`]).

use super::discovery::{DiscoveredFile, SkippedFile};
use super::file_pool::{self, Pass};
use super::publish::PublishError;
use super::revision::{FileRecord, SKIP_REASON_NOT_UTF8, SKIP_REASON_OVERSIZE};
use super::sample::{Sample, SampleError, sample_file, to_file_record};
use crate::model::ResourceLimits;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::util::Deadline;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Maximum number of changed files sampled and parsed concurrently in memory
/// before staging their rows and dropping their `content` and `evidence` buffers.
pub(super) const PASS2_CHUNK_FILES: usize = 256;

/// The sampling pass, as its log lines and time-budget error name it.
const SAMPLE_PASS: Pass = Pass {
    operation: "sample_batch",
    activity: "processing",
};

/// One file's raw sampling outcome before parsing.
enum Sampled {
    /// The file's content and identity.
    Sample(Sample),
    /// The file cannot be indexed and becomes a skipped row.
    Skipped(SkippedFile),
}

/// One file's Pass 2 chunk outcome: a parsed [`FileRecord`] or a per-file skip.
pub(super) enum ChunkOutcome {
    /// Fully sampled and parsed file record ready to stage and drop.
    Parsed(FileRecord),
    /// Skipped file whose bytes still match Pass 1's hash (or skipped in standalone test).
    Skipped(SkippedFile),
}

/// Shared Pass 2 context across bounded chunks of one publish attempt.
pub(super) struct Pass2ChunkContext<'a> {
    pub(super) total_files: usize,
    pub(super) pass1_hashes: &'a HashMap<&'a str, &'a str>,
    pub(super) limits: &'a ResourceLimits,
    pub(super) total_bytes: &'a AtomicU64,
    pub(super) sink: &'a dyn ProgressSink,
    pub(super) deadline: &'a Deadline,
}

/// Runs Pass 2 over `files_to_sample` in bounded [`PASS2_CHUNK_FILES`] chunks
/// on a single persistent scoped worker pool, invoking `on_chunk` after each
/// chunk completes so peak memory never scales beyond one 256-file chunk.
pub(super) fn run_pass2_chunks(
    files_to_sample: &[DiscoveredFile],
    ctx: &Pass2ChunkContext<'_>,
    mut on_chunk: impl FnMut(Vec<ChunkOutcome>) -> Result<(), PublishError>,
) -> Result<(), PublishError> {
    #[cfg(test)]
    let test_ctx = super::analyze::TestWorkerContext::capture();
    file_pool::with_pool(
        files_to_sample,
        ctx.deadline,
        SAMPLE_PASS,
        |global_idx, file| {
            #[cfg(test)]
            test_ctx.install();
            let outcome = sample_one(file, ctx.pass1_hashes, ctx.limits, ctx.total_bytes)?;
            let current =
                global_idx
                    .checked_add(1)
                    .ok_or(PublishError::SamplingIndexOutOfRange {
                        index: global_idx,
                        total: ctx.total_files,
                    })?;
            ctx.sink.emit(ProgressEvent::Sampling {
                phase: "publishing",
                current,
                total: ctx.total_files,
            });
            match outcome {
                Sampled::Skipped(skip) => Ok(ChunkOutcome::Skipped(skip)),
                Sampled::Sample(sample) => {
                    if let Some(&expected_hash) =
                        ctx.pass1_hashes.get(sample.relative_path.as_str())
                        && sample.identity.content_hash != expected_hash
                    {
                        return Err(PublishError::ChangedDuringSample {
                            path: sample.relative_path,
                        });
                    }
                    if let Some(elapsed) = ctx.deadline.exceeded() {
                        return Err(PublishError::time_budget_exceeded(
                            SAMPLE_PASS.activity,
                            &sample.relative_path,
                            elapsed,
                        ));
                    }
                    let record = to_file_record(sample, ctx.limits, ctx.deadline)?;
                    Ok(ChunkOutcome::Parsed(record))
                }
            }
        },
        |runner| {
            while let Some(outcomes) = runner.next_chunk(PASS2_CHUNK_FILES)? {
                on_chunk(outcomes)?;
            }
            Ok(())
        },
    )
}

/// Samples one file and claims its bytes against the import ceiling.
fn sample_one(
    file: &DiscoveredFile,
    pass1_hashes: &HashMap<&str, &str>,
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
        Err(SampleError::TooLarge { .. })
            if pass1_hashes.contains_key(file.relative_path.as_str()) =>
        {
            return Err(PublishError::ChangedDuringSample {
                path: file.relative_path.clone(),
            });
        }
        Err(error @ SampleError::TooLarge { .. }) => (error, SKIP_REASON_OVERSIZE),
        Err(error @ SampleError::NotUtf8 { .. }) => {
            if let SampleError::NotUtf8 {
                ref content_hash, ..
            } = error
                && pass1_hashes
                    .get(file.relative_path.as_str())
                    .is_some_and(|&expected| expected != content_hash)
            {
                return Err(PublishError::ChangedDuringSample {
                    path: file.relative_path.clone(),
                });
            }
            (error, SKIP_REASON_NOT_UTF8)
        }
    };
    Ok(Sampled::Skipped(SkippedFile {
        absolute_path: file.absolute_path.clone(),
        reason: error.to_string(),
        code,
    }))
}

/// Adds `byte_len` to the running import total across concurrent workers.
fn claim_bytes(total_bytes: &AtomicU64, byte_len: u64, limit: u64) -> Result<(), PublishError> {
    match total_bytes.fetch_update(Ordering::AcqRel, Ordering::Acquire, |prev| {
        prev.checked_add(byte_len).filter(|next| *next <= limit)
    }) {
        Ok(_) => Ok(()),
        Err(total_before) => match total_before.checked_add(byte_len) {
            Some(total) => Err(PublishError::ImportTooLarge { total, limit }),
            None => Err(PublishError::ImportTotalOverflow { limit }),
        },
    }
}

#[cfg(test)]
#[path = "sample_batch_tests.rs"]
mod tests;
