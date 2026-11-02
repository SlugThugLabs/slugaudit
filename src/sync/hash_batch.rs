//! Parallel streaming-hash worker pool for file discovery.
//!
//! Hashes discovered files on disk in fixed 64 KB chunks ([`hash_file`]) to produce
//! [`SourceIdentity`] without holding entire file texts in memory.

use super::discovery::{DiscoveredFile, SkippedFile};
use super::hash::{HashError, hash_file};
use super::publish::PublishError;
use super::revision_skipped::SKIP_REASON_OVERSIZE;
use super::sample::SampleError;
use crate::model::SourceIdentity;
use crate::util::Deadline;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;

enum HashOutcome {
    Ok(SourceIdentity),
    Skipped(SkippedFile),
}

struct HashBatch<'a> {
    discovered: &'a [DiscoveredFile],
    max_file_bytes: u64,
    deadline: &'a Deadline,
    total: usize,
    next_index: AtomicUsize,
    error_flag: AtomicBool,
    error_slot: Mutex<Option<PublishError>>,
}

impl<'a> HashBatch<'a> {
    fn abort(&self, error: PublishError) {
        if !self.error_flag.swap(true, Ordering::SeqCst) {
            let mut guard = crate::util::lock_or_recover(&self.error_slot, "hash_batch::abort");
            *guard = Some(error);
        } else {
            error.warn_sampling_superseded();
        }
    }

    fn send_outcome(
        &self,
        tx: &mpsc::Sender<(usize, HashOutcome)>,
        idx: usize,
        outcome: HashOutcome,
    ) -> bool {
        let file = &self.discovered[idx];
        match tx.send((idx, outcome)) {
            Ok(()) => true,
            Err(error) => {
                self.abort(PublishError::SamplingResultLost {
                    path: file.relative_path.clone(),
                    reason: error.to_string(),
                });
                false
            }
        }
    }

    fn run_worker(&self, tx: mpsc::Sender<(usize, HashOutcome)>) {
        while !self.error_flag.load(Ordering::Relaxed) {
            let idx = self.next_index.fetch_add(1, Ordering::Relaxed);
            if idx >= self.total {
                break;
            }
            if let Some(elapsed) = self.deadline.exceeded() {
                self.abort(PublishError::TimeBudgetExceeded {
                    path: format!(" while hashing {}", self.discovered[idx].relative_path),
                    elapsed_ms: elapsed.as_millis(),
                });
                return;
            }
            let file = &self.discovered[idx];
            match hash_file(
                &file.relative_path,
                &file.absolute_path,
                self.max_file_bytes,
            ) {
                Ok(identity) => {
                    if !self.send_outcome(&tx, idx, HashOutcome::Ok(identity)) {
                        return;
                    }
                }
                Err(HashError::TooLarge {
                    path,
                    observed,
                    limit,
                }) => {
                    let skip = SkippedFile {
                        absolute_path: path,
                        reason: format!(
                            "{} exceeds the {}-byte per-file ceiling (observed {} bytes)",
                            file.relative_path, limit, observed
                        ),
                        code: SKIP_REASON_OVERSIZE,
                    };
                    if !self.send_outcome(&tx, idx, HashOutcome::Skipped(skip)) {
                        return;
                    }
                }
                Err(HashError::Read { path, source }) => {
                    self.abort(PublishError::Sample(SampleError::Read { path, source }));
                    return;
                }
            }
        }
    }

    fn collect(
        self,
        rx: mpsc::Receiver<(usize, HashOutcome)>,
    ) -> Result<(Vec<SourceIdentity>, Vec<SkippedFile>), PublishError> {
        let mut results = Vec::with_capacity(self.total);
        while let Ok((idx, outcome)) = rx.recv() {
            results.push((idx, outcome));
        }
        let error = crate::util::lock_or_recover(&self.error_slot, "hash_batch::collect").take();
        if let Some(error) = error {
            return Err(error);
        }
        results.sort_by_key(|&(idx, _)| idx);
        let mut identities = Vec::with_capacity(results.len());
        let mut skips = Vec::new();
        for (_, outcome) in results {
            match outcome {
                HashOutcome::Ok(identity) => identities.push(identity),
                HashOutcome::Skipped(skip) => skips.push(skip),
            }
        }
        Ok((identities, skips))
    }
}

pub(super) fn hash_all_with_deadline(
    discovered: &[DiscoveredFile],
    max_file_bytes: u64,
    deadline: &Deadline,
) -> Result<(Vec<SourceIdentity>, Vec<SkippedFile>), PublishError> {
    let total = discovered.len();
    if total == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let parallelism = thread::available_parallelism().map_or(1, |n| n.get());
    let worker_count = total.min(parallelism.min(8));

    let (tx, rx) = mpsc::channel();
    let batch = HashBatch {
        discovered,
        max_file_bytes,
        deadline,
        total,
        next_index: AtomicUsize::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    };

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let tx = tx.clone();
            let batch = &batch;
            scope.spawn(move || batch.run_worker(tx));
        }
        drop(tx);
    });

    batch.collect(rx)
}

#[cfg(test)]
#[path = "hash_batch_tests.rs"]
mod tests;
