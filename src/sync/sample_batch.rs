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
use crate::util::Deadline;
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
    let worker_count = total.min(
        thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8),
    );

    let (tx, rx) = std::sync::mpsc::channel::<(usize, Result<Sample, SampleError>)>();

    let next_index = AtomicUsize::new(0);
    let total_bytes = AtomicU64::new(0);
    let error_flag = AtomicBool::new(false);
    let error_slot: Mutex<Option<PublishError>> = Mutex::new(None);

    thread::scope(|scope| {
        // Shared state is passed by `&` — legal inside `thread::scope`,
        // which guarantees every spawned thread is joined before the
        // scope closure returns, so all references stay valid for the
        // workers' lifetimes. This avoids wrapping each shared state
        // piece in `Arc` solely to please the borrow checker across
        // multiple `scope.spawn` calls in the loop.
        let next_index = &next_index;
        let total_bytes = &total_bytes;
        let error_flag = &error_flag;
        let error_slot = &error_slot;
        for _ in 0..worker_count {
            let tx = tx.clone();
            scope.spawn(move || {
                loop {
                    if error_flag.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some(elapsed) = deadline.exceeded() {
                        error_flag.store(true, Ordering::Release);
                        record_error(
                            error_slot,
                            PublishError::TimeBudgetExceeded {
                                elapsed_ms: elapsed.as_millis(),
                            },
                        );
                        return;
                    }
                    let idx = next_index.fetch_add(1, Ordering::AcqRel);
                    if idx >= total {
                        return;
                    }
                    match sample_file(&discovered[idx], limits) {
                        Ok(sample) => {
                            // `fetch_update` is the only correct primitive
                            // here: a plain `load`/`store` would race
                            // across workers and let two threads each
                            // push past the cap before either notices.
                            let limit_exceeded = total_bytes
                                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |prev| {
                                    let next = prev.saturating_add(sample.byte_len);
                                    if next > limits.max_total_import_bytes {
                                        None
                                    } else {
                                        Some(next)
                                    }
                                })
                                .is_err();
                            if limit_exceeded {
                                error_flag.store(true, Ordering::Release);
                                let observed = total_bytes
                                    .load(Ordering::Relaxed)
                                    .saturating_add(sample.byte_len);
                                record_error(
                                    error_slot,
                                    PublishError::ImportTooLarge {
                                        total: observed,
                                        limit: limits.max_total_import_bytes,
                                    },
                                );
                                return;
                            }
                            sink.emit(ProgressEvent::Sampling {
                                phase: "publishing",
                                current: idx + 1,
                                total,
                            });
                            // Send AFTER any cap/limit bookkeeping so a
                            // post-cap sample never lands in the result
                            // vector. `send` returns `Err` once the
                            // receiver is dropped (after this scope); the
                            // worker then exits cleanly.
                            if tx.send((idx, Ok(sample))).is_err() {
                                return;
                            }
                        }
                        // Per-file import ceiling: this file cannot be
                        // indexed at all. Skip it like discovery skips an
                        // unreadable file — record the reason through the
                        // channel, keep every other file going.
                        Err(error @ SampleError::TooLarge { .. }) => {
                            sink.emit(ProgressEvent::Sampling {
                                phase: "publishing",
                                current: idx + 1,
                                total,
                            });
                            if tx.send((idx, Err(error))).is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            error_flag.store(true, Ordering::Release);
                            record_error(error_slot, PublishError::Sample(error));
                            return;
                        }
                    }
                }
            });
        }
        // All senders live in worker closures; drop our `tx` clone once
        // the scope above closes so the receiver disconnects and
        // `recv` returns `Err` — terminating the drain cleanly.
        drop(tx);
    });

    // Drain in arrival order; the slot-by-index assignment below
    // re-establishes the original `discovered` ordering for downstream
    // consumers regardless of which worker finished which file first.
    let mut samples: Vec<Option<Sample>> = (0..total).map(|_| None).collect();
    let mut skipped: Vec<SkippedFile> = Vec::new();
    while let Ok((idx, result)) = rx.recv() {
        match result {
            Ok(sample) => samples[idx] = Some(sample),
            Err(error @ SampleError::TooLarge { .. }) => skipped.push(SkippedFile {
                absolute_path: discovered[idx].absolute_path.clone(),
                reason: error.to_string(),
            }),
            // Other sampling errors never reach the channel — they abort
            // via `error_slot` above. This arm is the exhaustiveness
            // safety net.
            Err(_) => {}
        }
    }

    if let Some(err) = error_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        return Err(err);
    }

    // A skipped (oversized) file leaves its slot empty; when no error was
    // recorded, every other slot is populated — a worker either sends a
    // sample, sends a skip, or records an error (which returned above).
    // `flatten` drops the skip slots and preserves discovered order.
    let samples = samples.into_iter().flatten().collect();
    // Deterministic report ordering, mirroring `discovery`'s sort of its
    // own skipped files (worker completion order is racy by design).
    skipped.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
    Ok((samples, skipped))
}

/// Records the first error from a parallel sampling run. Concurrent
/// workers race to call this; only the first winner's error reaches
/// callers because subsequent callers short-circuit on
/// `Option::is_some()`. `into_inner().ok()` removes the `PoisonError`
/// (no panic inside the mutex — sampled errors flow through
/// `record_error`, not panics).
fn record_error(slot: &Mutex<Option<PublishError>>, error: PublishError) {
    let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(error);
    }
}
