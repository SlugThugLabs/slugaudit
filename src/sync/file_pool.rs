// slugaudit-line-exception: approved-by=split-judge; reason=bounded deadline-checked scoped worker pool for ordered single-slice and multi-chunk file pass execution
//! Bounded scoped worker pool shared by Pass 1 (`hash_batch`) and Pass 2
//! (`sample_batch`).
//!
//! [`with_pool`] spawns at most `min(files.len(), available_parallelism,
//! MAX_CONCURRENT_BLOCKING_OPS)` OS threads once for an entire pass and keeps
//! them alive across bounded chunks (`BatchRunner::next_chunk`) so a multi-chunk
//! Pass 2 never spawns and tears down threads per 256-file chunk. [`run`]
//! processes an entire slice as one chunk on the same implementation.

use super::discovery::{DiscoveredFile, WalkedFile};
use super::publish::PublishError;
use super::publish_log;
use crate::util::{Deadline, MAX_CONCURRENT_BLOCKING_OPS, lock};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

/// An item processed by [`run`] or [`with_pool`] that names its
/// project-relative path for deadline and channel-loss diagnostics.
pub(super) trait PoolItem: Sync {
    /// Project-relative path of this file.
    fn relative_path(&self) -> &str;
}

impl PoolItem for DiscoveredFile {
    fn relative_path(&self) -> &str {
        &self.relative_path
    }
}

impl PoolItem for WalkedFile {
    fn relative_path(&self) -> &str {
        &self.relative_path
    }
}

/// Identifies one pass in its log lines and time-budget error.
#[derive(Debug, Clone, Copy)]
pub(super) struct Pass {
    /// The `operation` field of this pass's log lines.
    pub(super) operation: &'static str,
    /// The verb of the time-budget note: `" while {activity} {path}"`.
    pub(super) activity: &'static str,
}

type WorkerCommand<T> = (Arc<Batch>, mpsc::Sender<(usize, T)>);

/// Runs `per_file` once for every file in `files` on a bounded worker pool
/// and returns the outcomes in `files` order.
pub(super) fn run<I, T, F>(
    files: &[I],
    deadline: &Deadline,
    pass: Pass,
    per_file: F,
) -> Result<Vec<T>, PublishError>
where
    I: PoolItem,
    T: Send,
    F: Fn(usize, &I) -> Result<T, PublishError> + Sync,
{
    let total = files.len();
    with_pool(files, deadline, pass, per_file, |runner| {
        match runner.next_chunk(total)? {
            Some(outcomes) => Ok(outcomes),
            None => Ok(Vec::new()),
        }
    })
}

/// Spawns one bounded scoped worker pool for `files` and hands a
/// [`BatchRunner`] to `drive` so `files` can be processed in bounded chunks
/// on the same persistent worker threads.
pub(super) fn with_pool<'env, I, T, F, R>(
    files: &'env [I],
    deadline: &'env Deadline,
    pass: Pass,
    per_file: F,
    drive: impl FnOnce(&mut BatchRunner<'_, 'env, I, T>) -> Result<R, PublishError>,
) -> Result<R, PublishError>
where
    I: PoolItem,
    T: Send,
    F: Fn(usize, &I) -> Result<T, PublishError> + Sync,
{
    if files.is_empty() {
        let mut empty = BatchRunner::new(files, pass, Vec::new());
        return drive(&mut empty);
    }
    let count = worker_count(files.len(), pass);
    let shared = SharedPool {
        files,
        deadline,
        pass,
        per_file,
    };
    thread::scope(|scope| {
        let mut workers = Vec::with_capacity(count);
        for _ in 0..count {
            let (work_tx, work_rx) = mpsc::channel::<WorkerCommand<T>>();
            let shared_ref = &shared;
            scope.spawn(move || {
                while let Ok((batch, result_tx)) = work_rx.recv() {
                    shared_ref.run_worker(&batch, &result_tx);
                }
            });
            workers.push(work_tx);
        }
        let mut runner = BatchRunner::new(files, pass, workers);
        let out = drive(&mut runner);
        drop(runner);
        out
    })
}

/// Feeds sequential chunks of `files` to the persistent scoped worker pool.
pub(super) struct BatchRunner<'pool, 'env, I, T> {
    files: &'env [I],
    pass: Pass,
    cursor: usize,
    workers: Vec<mpsc::Sender<WorkerCommand<T>>>,
    _marker: std::marker::PhantomData<&'pool ()>,
}

impl<'env, I: PoolItem, T: Send> BatchRunner<'_, 'env, I, T> {
    fn new(files: &'env [I], pass: Pass, workers: Vec<mpsc::Sender<WorkerCommand<T>>>) -> Self {
        Self {
            files,
            pass,
            cursor: 0,
            workers,
            _marker: std::marker::PhantomData,
        }
    }

    /// Dispatches the next `chunk_size` files to the live worker threads and
    /// collects their outcomes in slice order, returning `Ok(None)` once all
    /// files have been processed.
    pub(super) fn next_chunk(&mut self, chunk_size: usize) -> Result<Option<Vec<T>>, PublishError> {
        if self.cursor >= self.files.len() || chunk_size == 0 {
            return Ok(None);
        }
        let start = self.cursor;
        // KEEP saturating: clamped immediately to self.files.len()
        let end = start.saturating_add(chunk_size).min(self.files.len());
        self.cursor = end;
        let Some(chunk) = self.files.get(start..end) else {
            return Ok(None);
        };
        let active_workers = chunk.len().min(self.workers.len());
        let (result_tx, result_rx) = mpsc::channel::<(usize, T)>();
        let batch = Arc::new(Batch::new(start, end, self.pass));
        for worker in self.workers.iter().take(active_workers) {
            if let Err(error) = worker.send((Arc::clone(&batch), result_tx.clone())) {
                let path = match chunk.first() {
                    Some(first) => first.relative_path().to_owned(),
                    None => String::new(),
                };
                return Err(PublishError::SamplingResultLost {
                    path,
                    reason: error.to_string(),
                });
            }
        }
        drop(result_tx);
        batch.collect(chunk, &result_rx).map(Some)
    }
}

/// How many workers to start for `total` files.
fn worker_count(total: usize, pass: Pass) -> usize {
    let parallelism = match thread::available_parallelism() {
        Ok(parallelism) => parallelism.get(),
        Err(error) => {
            tracing::warn!(
                operation = pass.operation,
                error = %error,
                "could not read available parallelism; running on one worker, \
                 so a large sync runs slower and may reach the sync deadline",
            );
            1
        }
    };
    total.min(parallelism).min(MAX_CONCURRENT_BLOCKING_OPS)
}

/// Immutable state shared across every chunk of a [`with_pool`] run.
struct SharedPool<'a, I, F> {
    files: &'a [I],
    deadline: &'a Deadline,
    pass: Pass,
    per_file: F,
}

impl<I: PoolItem, F> SharedPool<'_, I, F> {
    fn run_worker<T>(&self, batch: &Batch, tx: &mpsc::Sender<(usize, T)>)
    where
        F: Fn(usize, &I) -> Result<T, PublishError>,
    {
        while !batch.error_flag.load(Ordering::Acquire) {
            let idx = batch.next_index.fetch_add(1, Ordering::AcqRel);
            if idx >= batch.end {
                return;
            }
            let Some(file) = self.files.get(idx) else {
                return;
            };
            if let Some(elapsed) = self.deadline.exceeded() {
                batch.abort(PublishError::time_budget_exceeded(
                    self.pass.activity,
                    file.relative_path(),
                    elapsed,
                ));
                return;
            }
            let outcome = match (self.per_file)(idx, file) {
                Ok(outcome) => outcome,
                Err(error) => {
                    batch.abort(error);
                    return;
                }
            };
            let Some(local_idx) = idx.checked_sub(batch.start) else {
                batch.abort(PublishError::SamplingIndexOutOfRange {
                    index: idx,
                    total: batch.end,
                });
                return;
            };
            if let Err(error) = tx.send((local_idx, outcome)) {
                batch.abort(PublishError::SamplingResultLost {
                    path: file.relative_path().to_owned(),
                    reason: error.to_string(),
                });
                return;
            }
        }
    }
}

/// Per-chunk coordination state shared by workers executing `[start..end)`.
struct Batch {
    start: usize,
    end: usize,
    pass: Pass,
    next_index: AtomicUsize,
    error_flag: AtomicBool,
    error_slot: Mutex<Option<PublishError>>,
}

impl Batch {
    fn new(start: usize, end: usize, pass: Pass) -> Self {
        Self {
            start,
            end,
            pass,
            next_index: AtomicUsize::new(start),
            error_flag: AtomicBool::new(false),
            error_slot: Mutex::new(None),
        }
    }

    fn abort(&self, error: PublishError) {
        self.error_flag.store(true, Ordering::Release);
        match lock(&self.error_slot, "file_pool_error_slot") {
            Ok(mut slot) if slot.is_none() => *slot = Some(error),
            Ok(_) => publish_log::warn_discarded_worker_error(self.pass.operation, &error, None),
            Err(poisoned) => publish_log::warn_discarded_worker_error(
                self.pass.operation,
                &error,
                Some(&poisoned),
            ),
        }
    }

    fn collect<I: PoolItem, T>(
        &self,
        chunk: &[I],
        rx: &mpsc::Receiver<(usize, T)>,
    ) -> Result<Vec<T>, PublishError> {
        let mut slots: Vec<Option<T>> = chunk.iter().map(|_| None).collect();
        while let Ok((local_idx, outcome)) = rx.recv() {
            let Some(slot) = slots.get_mut(local_idx) else {
                return Err(PublishError::SamplingIndexOutOfRange {
                    index: local_idx,
                    total: chunk.len(),
                });
            };
            *slot = Some(outcome);
        }
        if let Some(error) = lock(&self.error_slot, "file_pool_error_slot")?.take() {
            return Err(error);
        }
        slots
            .into_iter()
            .zip(chunk)
            .map(|(slot, file)| {
                slot.ok_or_else(|| PublishError::SamplingResultLost {
                    path: file.relative_path().to_owned(),
                    reason: "no worker delivered an outcome for this file".to_owned(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "file_pool_tests.rs"]
mod tests;
