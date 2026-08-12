// slugaudit-line-exception: approved-by=agent; reason=single-file read/hash/classify, evidence caps, and the batch sampling loop (with its wall-clock deadline variant) are one cohesive sampling pipeline; splitting the 206-code-line module would scatter to_file_record's evidence capping away from the loop that feeds it
use super::analyze::analyze;
use super::discovery::{DiscoveredFile, FileKind};
use super::hash;
use super::publish::PublishError;
use super::revision::FileRecord;
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, ResourceLimits, SourceIdentity, SpanAvailability,
};
use crate::progress::{ProgressEvent, ProgressSink};
use crate::util::Deadline;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SampleError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is {size} bytes, exceeding the {limit}-byte per-file import ceiling")]
    TooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
}

/// One discovered file's on-disk state: content, hash, and whatever it
/// takes to classify it (binary vs. text, and whether reading it as text
/// required lossy UTF-8 conversion).
#[derive(Debug)]
pub struct Sample {
    pub relative_path: String,
    pub is_binary: bool,
    pub content: Option<String>,
    /// True when `content` required lossy UTF-8 conversion (invalid byte
    /// sequences replaced with U+FFFD). Recorded as evidence rather than
    /// silently accepted — see `to_file_record`.
    utf8_lossy: bool,
    pub identity: SourceIdentity,
    pub byte_len: u64,
}

/// # Errors
///
/// Returns an error if `file.absolute_path` can't be read, or if its size
/// exceeds `limits.max_file_bytes`.
pub fn sample_file(file: &DiscoveredFile, limits: &ResourceLimits) -> Result<Sample, SampleError> {
    let metadata = std::fs::metadata(&file.absolute_path).map_err(|source| SampleError::Read {
        path: file.absolute_path.clone(),
        source,
    })?;
    let size = metadata.len();
    if size > limits.max_file_bytes {
        return Err(SampleError::TooLarge {
            path: file.absolute_path.clone(),
            size,
            limit: limits.max_file_bytes,
        });
    }

    let bytes = std::fs::read(&file.absolute_path).map_err(|source| SampleError::Read {
        path: file.absolute_path.clone(),
        source,
    })?;
    // Re-check after the read: the file may have grown between stat and
    // read (TOCTOU). Prefer failing closed over indexing a huge buffer.
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(SampleError::TooLarge {
            path: file.absolute_path.clone(),
            size: bytes.len() as u64,
            limit: limits.max_file_bytes,
        });
    }

    let is_binary = file.kind == FileKind::Binary;
    let (content, utf8_lossy) = if is_binary {
        (None, false)
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(text) => (Some(text.to_owned()), false),
            Err(_) => (Some(String::from_utf8_lossy(&bytes).into_owned()), true),
        }
    };
    Ok(Sample {
        relative_path: file.relative_path.clone(),
        is_binary,
        byte_len: bytes.len() as u64,
        identity: hash::hash_bytes(&file.relative_path, &bytes),
        content,
        utf8_lossy,
    })
}

fn utf8_lossy_evidence() -> EvidenceItem {
    EvidenceItem {
        key: "encoding:0".to_owned(),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::SourceContent,
        span: SpanAvailability::DerivedEvidence,
        payload: json!({
            "message": "file contains invalid UTF-8 byte sequences; replaced with U+FFFD during indexing",
            "severity": "warning",
        }),
    }
}

/// Runs pack analysis on a sampled file and folds the result into a
/// storable `FileRecord`, appending an encoding diagnostic when the
/// content required lossy UTF-8 conversion and capping evidence to
/// `limits.evidence`.
pub fn to_file_record(sample: Sample, limits: &ResourceLimits) -> FileRecord {
    let mut parsed = analyze(&sample.relative_path, sample.content.as_deref());
    if sample.utf8_lossy {
        parsed.evidence.push(utf8_lossy_evidence());
    }
    apply_evidence_limits(&mut parsed.evidence, limits);
    FileRecord {
        relative_path: sample.relative_path,
        is_binary: sample.is_binary,
        content: sample.content,
        identity: sample.identity,
        byte_len: sample.byte_len,
        language: parsed.language,
        language_detected: parsed.language_detected,
        run: parsed.run,
        evidence: parsed.evidence,
    }
}

/// Reserved out of `max_payload_bytes_per_file` so the truncation marker
/// itself never pushes a file's evidence over the cap it is reporting on.
const TRUNCATION_MARKER_RESERVE_BYTES: usize = 512;

/// Enforces all three evidence caps in one pass: per-item count, per-item
/// payload bytes, and — the one the old truncate-then-retain version never
/// checked — cumulative payload bytes across every kept item. A file with
/// many small-but-not-tiny items (each under the per-item cap) could still
/// serialize to a multi-megabyte evidence blob; this is what actually
/// caught the 5,326,990-byte failure. Whenever anything is discarded, an
/// explicit `Diagnostic` evidence item records it — the AI sees a "this
/// file's evidence was truncated" signal instead of silently incomplete data.
pub(crate) fn apply_evidence_limits(items: &mut Vec<EvidenceItem>, limits: &ResourceLimits) {
    let cap = limits.evidence;
    let original_len = items.len();
    let max_items = cap.max_items_per_file.saturating_sub(1);
    let max_bytes = cap
        .max_payload_bytes_per_file
        .saturating_sub(TRUNCATION_MARKER_RESERVE_BYTES);

    let mut kept = Vec::with_capacity(original_len.min(cap.max_items_per_file));
    let mut cumulative_bytes = 0_usize;
    let mut oversized_dropped = 0_usize;
    let mut limit_truncated = false;

    for item in items.drain(..) {
        if kept.len() >= max_items {
            limit_truncated = true;
            break;
        }
        let payload_bytes =
            serde_json::to_vec(&item.payload).map_or(usize::MAX, |bytes| bytes.len());
        if payload_bytes > cap.max_payload_bytes_per_item {
            oversized_dropped += 1;
            continue;
        }
        if cumulative_bytes.saturating_add(payload_bytes) > max_bytes {
            limit_truncated = true;
            break;
        }
        cumulative_bytes += payload_bytes;
        kept.push(item);
    }

    let kept_len = kept.len();
    *items = kept;
    if limit_truncated || oversized_dropped > 0 {
        items.push(truncation_evidence(
            original_len,
            kept_len,
            oversized_dropped,
        ));
    }
}

fn truncation_evidence(
    original_count: usize,
    kept_count: usize,
    oversized_dropped: usize,
) -> EvidenceItem {
    let dropped_for_cap = original_count
        .saturating_sub(kept_count)
        .saturating_sub(oversized_dropped);
    EvidenceItem {
        key: "evidence:truncated".to_owned(),
        kind: EvidenceKind::Diagnostic,
        origin: EvidenceOrigin::SourceContent,
        span: SpanAvailability::DerivedEvidence,
        payload: json!({
            "message": format!(
                "evidence limits discarded records: {oversized_dropped} item(s) exceeded the \
                 per-item byte cap, {dropped_for_cap} item(s) dropped by the item-count or \
                 cumulative-byte cap"
            ),
            "severity": "warning",
            "original_item_count": original_count,
            "kept_item_count": kept_count,
        }),
    }
}

/// Samples every discovered file, accumulating the total byte count and
/// failing closed if the project-wide import ceiling is exceeded — or if
/// an explicit [`Deadline`] (checked once per file) is spent, so a
/// pathological repo (a huge tree, a hung filesystem) fails closed with
/// [`PublishError::TimeBudgetExceeded`] instead of stalling the publish
/// forever. The deadline is created by the caller so the whole operation
/// shares one budget across sampling, diffing, revalidation, and retries.
/// Kept here (next to `sample_file`) rather than in `publish` — it's a
/// sampling concern, not an orchestration one.
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
) -> Result<Vec<Sample>, PublishError> {
    let total = discovered.len();
    if total == 0 {
        return Ok(Vec::new());
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
                    let raw_result = sample_file(&discovered[idx], limits);
                    match raw_result {
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
    while let Ok((idx, result)) = rx.recv() {
        if let Ok(sample) = result {
            samples[idx] = Some(sample);
        }
    }

    if let Some(err) = error_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        return Err(err);
    }
    Ok(samples
        .into_iter()
        .map(|slot| slot.expect("every slot populated when no error was recorded"))
        .collect())
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

#[cfg(test)]
#[path = "sample_tests.rs"]
mod tests;
