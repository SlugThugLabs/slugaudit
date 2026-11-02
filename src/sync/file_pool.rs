//! The bounded worker pool both passes of a full sync run on: Pass 1
//! stream-hashes every discovered file (`hash_batch`), Pass 2 samples the
//! files that changed (`sample_batch`).
//!
//! The pool owns everything the two passes share: claiming file indices,
//! the once-per-file deadline check, first-error-wins abort, and putting the
//! per-file outcomes back into discovered order. A pass supplies only what
//! happens to one file, as a closure.
//!
//! Worker count is `min(files, available_parallelism,
//! MAX_CONCURRENT_BLOCKING_OPS)`: never more threads than files, never more
//! than the cores, and never more than the server's blocking-operation
//! permits, so one sync does not outnumber the other tool calls.
//!
//! Shared state is passed by `&`, which `thread::scope` allows: every
//! spawned thread is joined before the scope returns.

use super::discovery::DiscoveredFile;
use super::publish::PublishError;
use super::publish_log;
use crate::server_runner::MAX_CONCURRENT_BLOCKING_OPS;
use crate::util::{Deadline, LockPoisoned, lock};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

/// What names one pass in its log lines and its time-budget error.
#[derive(Debug, Clone, Copy)]
pub(super) struct Pass {
    /// The `operation` field of this pass's log lines.
    pub(super) operation: &'static str,
    /// The verb of the time-budget note: `" while {activity} {path}"`.
    pub(super) activity: &'static str,
}

/// Runs `per_file` once for every file in `files` on a bounded worker pool
/// and returns the outcomes in `files` order.
///
/// `per_file` receives the file's index in `files` and the file. An `Err`
/// from it aborts the run: the other workers stop at their next file, and
/// the first error recorded is the one returned. A later worker's distinct
/// failure is logged once at `warn` rather than dropped. The deadline is
/// checked once per file, after the index is claimed, so the
/// budget-exceeded error names the file that was next in line.
///
/// # Errors
///
/// The first error `per_file` returned, a spent time budget, a worker whose
/// outcome could not be delivered, or a poisoned error slot.
pub(super) fn run<T, F>(
    files: &[DiscoveredFile],
    deadline: &Deadline,
    pass: Pass,
    per_file: F,
) -> Result<Vec<T>, PublishError>
where
    T: Send,
    F: Fn(usize, &DiscoveredFile) -> Result<T, PublishError> + Sync,
{
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let worker_count = worker_count(files.len(), pass);
    let (tx, rx) = mpsc::channel::<(usize, T)>();
    let pool = Pool {
        files,
        deadline,
        pass,
        per_file,
        next_index: AtomicUsize::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    };
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let tx = tx.clone();
            let pool = &pool;
            scope.spawn(move || pool.run_worker(&tx));
        }
        // Every sender now lives in a worker; dropping this one lets the
        // drain's `recv` end once the last worker exits.
        drop(tx);
    });
    pool.collect(&rx)
}

/// The error for a spent time budget while `relative_path` was next in line,
/// naming the file and the elapsed time the same way for every caller.
pub(super) fn time_budget_error(
    activity: &str,
    relative_path: &str,
    elapsed: Duration,
) -> PublishError {
    PublishError::TimeBudgetExceeded {
        path: format!(" while {activity} {relative_path}"),
        elapsed_ms: elapsed.as_millis(),
    }
}

/// How many workers to start for `total` files. When the core count cannot
/// be read the pass still runs, on one worker: the result is the same, only
/// slower, so the failure is logged and the pass continues.
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

/// Shared state for one run of the pool.
struct Pool<'a, F> {
    files: &'a [DiscoveredFile],
    deadline: &'a Deadline,
    pass: Pass,
    per_file: F,
    next_index: AtomicUsize,
    error_flag: AtomicBool,
    error_slot: Mutex<Option<PublishError>>,
}

impl<F> Pool<'_, F> {
    /// One worker's loop: claim the next index, check the deadline, run the
    /// pass on that file and send the outcome. Stops on the shared error
    /// flag, at the end of the file set, or after recording an error.
    fn run_worker<T>(&self, tx: &mpsc::Sender<(usize, T)>)
    where
        F: Fn(usize, &DiscoveredFile) -> Result<T, PublishError>,
    {
        while !self.error_flag.load(Ordering::Acquire) {
            let idx = self.next_index.fetch_add(1, Ordering::AcqRel);
            let Some(file) = self.files.get(idx) else {
                return;
            };
            if let Some(elapsed) = self.deadline.exceeded() {
                self.abort(time_budget_error(
                    self.pass.activity,
                    &file.relative_path,
                    elapsed,
                ));
                return;
            }
            let outcome = match (self.per_file)(idx, file) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.abort(error);
                    return;
                }
            };
            // The receiver outlives the worker scope, so a failed send is a
            // broken invariant: the outcome would be lost and the result
            // would silently miss a file. It fails the run instead.
            if let Err(error) = tx.send((idx, outcome)) {
                self.abort(PublishError::SamplingResultLost {
                    path: file.relative_path.clone(),
                    reason: error.to_string(),
                });
                return;
            }
        }
    }

    /// Stops the other workers and records `error` when it is the first;
    /// a later error is logged once instead.
    fn abort(&self, error: PublishError) {
        self.error_flag.store(true, Ordering::Release);
        match lock(&self.error_slot, "file_pool_error_slot") {
            Ok(mut slot) if slot.is_none() => *slot = Some(error),
            Ok(_) => warn_superseded(self.pass, &error),
            // The run still fails: `collect` locks the same slot and returns
            // the poisoned-lock error.
            Err(poisoned) => warn_unrecorded(self.pass, &error, &poisoned),
        }
    }

    /// Drains the outcomes in arrival order and puts each into its file's
    /// slot, so the result follows `files` order whichever worker finished
    /// first.
    fn collect<T>(&self, rx: &mpsc::Receiver<(usize, T)>) -> Result<Vec<T>, PublishError> {
        let mut slots: Vec<Option<T>> = self.files.iter().map(|_| None).collect();
        while let Ok((idx, outcome)) = rx.recv() {
            // The index arrives over a channel, so it is checked like input:
            // an out-of-range one fails the run, never clamped or skipped.
            let Some(slot) = slots.get_mut(idx) else {
                return Err(PublishError::SamplingIndexOutOfRange {
                    index: idx,
                    total: self.files.len(),
                });
            };
            *slot = Some(outcome);
        }
        if let Some(error) = lock(&self.error_slot, "file_pool_error_slot")?.take() {
            return Err(error);
        }
        // With no error recorded every index was claimed and delivered; an
        // empty slot would be a lost outcome, and fails the run.
        slots
            .into_iter()
            .zip(self.files)
            .map(|(slot, file)| {
                slot.ok_or_else(|| PublishError::SamplingResultLost {
                    path: file.relative_path.clone(),
                    reason: "no worker delivered an outcome for this file".to_owned(),
                })
            })
            .collect()
    }
}

/// Logs a failure that lost the first-error race: the pass, the failure's
/// kind and path, and its `Display`.
fn warn_superseded(pass: Pass, error: &PublishError) {
    let (error_kind, path) = publish_log::failure_fields(error);
    tracing::warn!(
        operation = pass.operation,
        error_kind,
        path = ?path,
        error = %error,
        "a later worker failure was discarded after the first error was recorded",
    );
}

/// Logs a failure the poisoned error slot could not hold.
fn warn_unrecorded(pass: Pass, error: &PublishError, poisoned: &LockPoisoned) {
    let (error_kind, path) = publish_log::failure_fields(error);
    tracing::warn!(
        operation = pass.operation,
        error_kind,
        path = ?path,
        error = %error,
        slot_error = %poisoned,
        "a worker failure could not be recorded; the run fails with the poisoned slot",
    );
}

#[cfg(test)]
#[path = "file_pool_tests.rs"]
mod tests;
