//! Parallel batch sampling for a full publish.
//!
//! `sample_all_with_deadline` is the bounded worker-pool loop that feeds
//! `super::sample::to_file_record` — split out of `sample.rs` so the
//! per-file API (read/hash/classify, evidence capping) and the batch loop
//! each stay under the source-size cap. The loop is a sampling concern,
//! not an orchestration one; `publish_attempt` calls it directly.
//!
//! Files that individually exceed `limits.max_file_bytes` are **skipped
//! per-file** rather than failing the publish: such a file cannot be
//! indexed at all, and letting one oversized generated bundle make the
//! whole project unqueryable is worse than reporting it. The skip reason
//! is returned (sorted by path for deterministic report ordering) so the
//! caller can surface it in the publish report, mirroring how discovery
//! skips unreadable files. Only the project-wide total-byte ceiling and
//! hard errors remain fatal.

use super::discovery::{DiscoveredFile, SkippedFile};
use super::publish::PublishError;
use super::sample::{Sample, SampleError, sample_file};
use crate::model::ResourceLimits;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::util::{Deadline, lock_or_recover};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;

/// Samples every discovered file, accumulating the total byte count and
/// failing closed if the project-wide import ceiling is exceeded — or if
/// an explicit [`Deadline`] (checked once per file) is spent, so a
/// pathological repo (a huge tree, a hung filesystem) fails closed with
/// [`PublishError::TimeBudgetExceeded`] instead of stalling the publish
/// forever. The deadline is created by the caller so the whole operation
/// shares one budget across sampling, diffing, revalidation, and retries.
/// Kept here (next to `sample_file`) rather than in `publish` — it's a
/// sampling concern, not an orchestration one. Per-file oversize skips
/// are documented at the module level.
///
/// Parallelized across a bounded worker pool (`min(N,
/// available_parallelism)`) since the synchronous per-file loop was the
/// C3 audit's flagship bottleneck: a 60 s sync deadline compounded with
/// fully serialized sampling meant a 60 k-file first import was the
/// single most likely production failure mode. The atomic
/// counter/dispatcher preserves the original index in `samples[idx]`,
/// so downstream consumers (`manifest::compare`, `aggregate_manifest_hash`,
/// `build_upserts_and_deletions`) see the same byte-for-byte ordering
/// they always saw — `aggregate_manifest_hash` sorts internally too, so
/// the manifest hash stays deterministic. Per-file `Sampling` events
/// arrive at the sink in worker-completion order rather than file-index
/// order; the receiving sink (`McpProgressSink` / `NoopProgressSink`)
/// already tolerates out-of-order updates because file sampling is
/// intrinsically parallel and the `i/N` ratio is what consumers display.
///
/// The byte-cap check uses `fetch_update` so the limit is enforced
/// across concurrent workers: a worker that would push the total above
/// `max_total_import_bytes` fails the CAS, signals the others via
/// `error_flag`, and returns. The first worker to fail sets the error
/// slot; later workers see the flag and exit without touching the
/// slot, so the user sees the first concrete failure rather than
/// whichever race-loser happened to win.
pub(super) fn sample_all_with_deadline(
    discovered: &[DiscoveredFile],
    limits: &ResourceLimits,
    sink: &dyn ProgressSink,
    deadline: &Deadline,
) -> Result<(Vec<Sample>, Vec<SkippedFile>), PublishError> {
    let total = discovered.len();
    if total == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    // Bounded worker pool. Capping at `total` avoids spawning more
    // threads than files for the common small-input case; capping at
    // `available_parallelism` (capped at 8 to match the run_blocking
    // semaphore's permit cap, so we don't outnumber other concurrent
    // tool-call concurrency points) avoids oversubscribing.
    let parallelism = thread::available_parallelism().map_or_else(
        |error| {
            tracing::warn!(
                error = %error,
                "could not read available parallelism; sampling on one worker, \
                 so a large first import runs slower and may reach the sync deadline",
            );
            1
        },
        |n| n.get(),
    );
    let worker_count = total.min(parallelism.min(8));

    let (tx, rx) = std::sync::mpsc::channel::<(usize, Outcome)>();
    let batch = Batch {
        discovered,
        limits,
        deadline,
        total,
        next_index: AtomicUsize::new(0),
        total_bytes: AtomicU64::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    };

    // Shared state is passed by `&` — legal inside `thread::scope`,
    // which guarantees every spawned thread is joined before the scope
    // closure returns, so all references stay valid for the workers'
    // lifetimes. This avoids wrapping each shared state piece in `Arc`
    // solely to please the borrow checker across multiple `scope.spawn`
    // calls in the loop.
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let tx = tx.clone();
            let batch = &batch;
            scope.spawn(move || batch.run_worker(tx, sink));
        }
        // All senders live in worker closures; drop our `tx` clone once
        // the scope above closes so the receiver disconnects and
        // `recv` returns `Err` — terminating the drain cleanly.
        drop(tx);
    });

    batch.collect(rx)
}

/// What a worker sends the drain for one file: the sample, or the reason the
/// file was skipped. Every other sampling error aborts through `error_slot`
/// and never reaches the channel, so the drain has no error arm to ignore.
type Outcome = Result<Sample, String>;

/// Shared state for one parallel sampling run, with the worker loop and
/// the result drain as methods so each stays small; `sample_all_with_deadline`
/// owns the pool setup and nothing else.
struct Batch<'a> {
    discovered: &'a [DiscoveredFile],
    limits: &'a ResourceLimits,
    deadline: &'a Deadline,
    total: usize,
    next_index: AtomicUsize,
    total_bytes: AtomicU64,
    error_flag: AtomicBool,
    error_slot: Mutex<Option<PublishError>>,
}

impl Batch<'_> {
    /// One worker's loop: claim indices, sample, do byte-cap bookkeeping,
    /// and send results in arrival order. Exits on the shared error flag,
    /// an exhausted deadline, claimed-index overflow, or a dropped
    /// receiver.
    fn run_worker(&self, tx: std::sync::mpsc::Sender<(usize, Outcome)>, sink: &dyn ProgressSink) {
        loop {
            if self.error_flag.load(Ordering::Acquire) {
                return;
            }
            let idx = self.next_index.fetch_add(1, Ordering::AcqRel);
            if idx >= self.total {
                return;
            }
            // The deadline is checked after claiming the index so the
            // budget-exceeded error can name the file that was next in
            // line — the same "once per file" semantics as before, just
            // with a nameable victim.
            if let Some(elapsed) = self.deadline.exceeded() {
                self.abort(PublishError::TimeBudgetExceeded {
                    path: format!(" while processing {}", self.discovered[idx].relative_path),
                    elapsed_ms: elapsed.as_millis(),
                });
                return;
            }
            match sample_file(&self.discovered[idx], self.limits) {
                Ok(sample) => {
                    if !self.claim_bytes(sample.byte_len) {
                        return;
                    }
                    self.emit_sampling(sink, idx);
                    // Send AFTER any cap/limit bookkeeping so a post-cap
                    // sample never lands in the result vector.
                    if !self.deliver(&tx, idx, Ok(sample)) {
                        return;
                    }
                }
                // Per-file import ceiling: this file cannot be indexed at
                // all. Skip it like discovery skips an unreadable file —
                // record the reason through the channel, keep every other
                // file going.
                Err(error @ SampleError::TooLarge { .. }) => {
                    self.emit_sampling(sink, idx);
                    if !self.deliver(&tx, idx, Err(error.to_string())) {
                        return;
                    }
                }
                Err(error) => {
                    self.abort(PublishError::Sample(error));
                    return;
                }
            }
        }
    }

    /// Adds `byte_len` to the running import total. `fetch_update` is the
    /// only correct primitive: a plain load/store would race across
    /// workers and let two threads each push past the cap before either
    /// notices. Returns false (having recorded `ImportTooLarge` and set
    /// the flag) when the add would exceed `max_total_import_bytes`.
    fn claim_bytes(&self, byte_len: u64) -> bool {
        let claimed = self
            .total_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |prev| {
                let next = prev.saturating_add(byte_len);
                (next <= self.limits.max_total_import_bytes).then_some(next)
            });
        if let Err(total_before) = claimed {
            // `Err` carries the total this claim was refused against.
            self.abort(PublishError::ImportTooLarge {
                total: total_before.saturating_add(byte_len),
                limit: self.limits.max_total_import_bytes,
            });
        }
        claimed.is_ok()
    }

    /// Sends one outcome to the drain. The receiver outlives the worker
    /// scope, so a failed send is a broken invariant, not a normal exit:
    /// the outcome would be lost and the result would silently miss a file.
    /// It aborts the run with the cause instead; returns `false` then so the
    /// worker stops.
    fn deliver(
        &self,
        tx: &std::sync::mpsc::Sender<(usize, Outcome)>,
        idx: usize,
        outcome: Outcome,
    ) -> bool {
        let Some(file) = self.discovered.get(idx) else {
            self.abort(self.index_out_of_range(idx));
            return false;
        };
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

    /// The typed failure for an index that is not a position in the
    /// discovered set. Checked at every entry point into the batch state so
    /// a bad index can never panic the process.
    fn index_out_of_range(&self, index: usize) -> PublishError {
        PublishError::SamplingIndexOutOfRange {
            index,
            total: self.discovered.len(),
        }
    }

    /// Signals other workers and records the first error callers see.
    fn abort(&self, error: PublishError) {
        self.error_flag.store(true, Ordering::Release);
        record_error(&self.error_slot, error);
    }

    fn emit_sampling(&self, sink: &dyn ProgressSink, idx: usize) {
        sink.emit(ProgressEvent::Sampling {
            phase: "publishing",
            current: idx + 1,
            total: self.total,
        });
    }

    /// Drains in arrival order; the slot-by-index assignment re-establishes
    /// the original `discovered` ordering for downstream consumers
    /// regardless of which worker finished which file first.
    fn collect(
        &self,
        rx: std::sync::mpsc::Receiver<(usize, Outcome)>,
    ) -> Result<(Vec<Sample>, Vec<SkippedFile>), PublishError> {
        let mut samples: Vec<Option<Sample>> = (0..self.total).map(|_| None).collect();
        let mut skipped: Vec<SkippedFile> = Vec::new();
        while let Ok((idx, result)) = rx.recv() {
            // The index arrives over a channel from a worker, so treat it as
            // unverified input: an out-of-range one is a contract violation
            // that must fail the call, never be skipped or clamped.
            let Some(slot) = samples.get_mut(idx) else {
                return Err(self.index_out_of_range(idx));
            };
            match result {
                Ok(sample) => *slot = Some(sample),
                Err(reason) => {
                    let Some(file) = self.discovered.get(idx) else {
                        return Err(self.index_out_of_range(idx));
                    };
                    skipped.push(SkippedFile {
                        absolute_path: file.absolute_path.clone(),
                        reason,
                    });
                }
            }
        }
        if let Some(err) = lock_or_recover(&self.error_slot).take() {
            return Err(err);
        }
        // A skipped (oversized) file leaves its slot empty; when no error
        // was recorded, every other slot is populated — a worker either
        // sends a sample, sends a skip, or records an error (which
        // returned above). `flatten` drops the skip slots and preserves
        // discovered order.
        let samples = samples.into_iter().flatten().collect();
        // Deterministic report ordering, mirroring `discovery`'s sort of
        // its own skipped files (worker completion order is racy by design).
        skipped.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
        Ok((samples, skipped))
    }
}

/// Records the first error from a parallel sampling run. Concurrent
/// workers race to call this; only the first winner's error reaches
/// callers because subsequent callers short-circuit on
/// `Option::is_some()`. Locks through `lock_or_recover` (the project-
/// wide helper) so a poisoned mutex is recovered with an `error!` log
/// instead of silently unwrapped.
///
/// A later worker's distinct failure is still logged once at `warn` — with
/// a static operation, the error's category, and only a redacted path id —
/// so it is not silently discarded just because the first failure wins the
/// return value. Its raw `Display` never enters the log: several variants
/// embed a path.
fn record_error(slot: &Mutex<Option<PublishError>>, error: PublishError) {
    let mut guard = lock_or_recover(slot);
    if guard.is_none() {
        *guard = Some(error);
        return;
    }
    error.warn_sampling_superseded();
}

#[cfg(test)]
#[path = "sample_batch_tests.rs"]
mod tests;
