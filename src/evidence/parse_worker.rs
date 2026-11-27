// slugaudit-line-exception: approved-by=split-judge; reason=process-wide bounded pool of persistent parse workers with timeout-bounded reply waits and wedged-slot retirement
//! Process-wide bounded pool of persistent worker threads for bounded
//! `tree_sitter_language_pack::process()` runs on non-Rust languages.
//!
//! `tree-sitter-language-pack` caches one `tree_sitter::Parser` per language in a
//! `thread_local! PARSER_CACHE` and does not call `Parser::reset()` after a
//! progress-callback timeout cancellation. Keeping a process-wide pool of up to
//! [`MAX_PARSE_WORKERS`] persistent threads avoids spawning and tearing down OS
//! threads every sync chunk, retires any worker whose parse returns
//! [`PackError::ParseTimeout`] or panics, and bounds `reply_rx` with
//! `recv_timeout` so a wedged external scanner is discarded cleanly instead of
//! blocking the caller forever.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Condvar, LazyLock, Mutex, PoisonError};
use std::thread::Builder;
#[cfg(test)]
use std::thread::ThreadId;
use std::time::Duration;
use tree_sitter_language_pack::{Error as PackError, ProcessConfig, ProcessResult, process};

/// Maximum number of concurrent persistent `process()` worker threads in the
/// server process.
pub(super) const MAX_PARSE_WORKERS: usize = 8;

/// Extra grace window beyond `timeout_ms` allowed for tree-sitter's progress
/// callback to unwind and send its `ParseTimeout` reply before the caller
/// declares the worker wedged and abandons its slot.
const WORKER_REPLY_GRACE: Duration = Duration::from_secs(2);

struct ParseJob {
    source: Box<str>,
    config: ProcessConfig,
    #[cfg(test)]
    simulated_delay: Option<Duration>,
}

enum WorkerFailure {
    Pack(PackError),
    Panicked,
}

type WorkerReply = Result<Box<ProcessResult>, WorkerFailure>;

struct ParseWorker {
    job_tx: SyncSender<ParseJob>,
    reply_rx: Receiver<WorkerReply>,
    #[cfg(test)]
    thread_id: ThreadId,
}

impl ParseWorker {
    fn spawn() -> Result<Self, PackError> {
        let (job_tx, job_rx) = sync_channel::<ParseJob>(1);
        let (reply_tx, reply_rx) = sync_channel::<WorkerReply>(1);
        let handle = Builder::new()
            .name("slugaudit-parse".to_owned())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    #[cfg(test)]
                    if let Some(delay) = job.simulated_delay {
                        std::thread::sleep(delay);
                    }
                    let outcome =
                        catch_unwind(AssertUnwindSafe(|| process(&job.source, &job.config)));
                    let retire =
                        matches!(outcome, Err(_) | Ok(Err(PackError::ParseTimeout { .. })));
                    let reply = match outcome {
                        Ok(Ok(result)) => Ok(Box::new(result)),
                        Ok(Err(err)) => Err(WorkerFailure::Pack(err)),
                        Err(_panic_payload) => Err(WorkerFailure::Panicked),
                    };
                    if reply_tx.send(reply).is_err() || retire {
                        break;
                    }
                }
            })
            .map_err(|error| {
                tracing::error!(
                    operation = "parse_thread",
                    error_kind = ?error.kind(),
                    "could not start the thread for a bounded parse",
                );
                PackError::Config("could not start the thread for a bounded parse".to_owned())
            })?;
        #[cfg(test)]
        let thread_id = handle.thread().id();
        // Detach the OS handle: healthy workers exit once `job_tx` drops, and a
        // wedged worker abandoned on `RecvTimeoutError::Timeout` must never
        // block `Drop` in `JoinHandle::join()`.
        drop(handle);
        Ok(Self {
            job_tx,
            reply_rx,
            #[cfg(test)]
            thread_id,
        })
    }
}

struct PoolState {
    idle: Vec<ParseWorker>,
    checked_out: usize,
}

pub(super) struct WorkerPool {
    state: Mutex<PoolState>,
    available: Condvar,
    reply_grace: Duration,
}

impl WorkerPool {
    pub(super) fn new() -> Self {
        Self::with_grace(WORKER_REPLY_GRACE)
    }

    pub(super) fn with_grace(reply_grace: Duration) -> Self {
        Self {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                checked_out: 0,
            }),
            available: Condvar::new(),
            reply_grace,
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, PoolState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => PoisonError::into_inner(poisoned),
        }
    }

    fn acquire(&self, timeout_ms: u64) -> Result<SlotLease<'_>, PackError> {
        let wait_limit = Duration::from_millis(timeout_ms);
        let deadline = std::time::Instant::now().checked_add(wait_limit);
        let mut guard = self.lock_state();
        while guard.checked_out >= MAX_PARSE_WORKERS {
            let Some(limit) = deadline else {
                guard = match self.available.wait(guard) {
                    Ok(g) => g,
                    Err(poisoned) => PoisonError::into_inner(poisoned),
                };
                continue;
            };
            let Some(remaining) = limit.checked_duration_since(std::time::Instant::now()) else {
                return Err(PackError::ParseTimeout { timeout_ms });
            };
            if remaining.is_zero() {
                return Err(PackError::ParseTimeout { timeout_ms });
            }
            let (next_guard, wait_res) = match self.available.wait_timeout(guard, remaining) {
                Ok(pair) => pair,
                Err(poisoned) => PoisonError::into_inner(poisoned),
            };
            guard = next_guard;
            if wait_res.timed_out() && guard.checked_out >= MAX_PARSE_WORKERS {
                return Err(PackError::ParseTimeout { timeout_ms });
            }
        }
        // KEEP saturating: checked_out is strictly below MAX_PARSE_WORKERS (8) above
        guard.checked_out = guard.checked_out.saturating_add(1);
        let worker = guard.idle.pop();
        drop(guard);
        Ok(SlotLease { pool: self, worker })
    }

    fn run_job(&self, job: ParseJob, timeout_ms: u64) -> Result<ProcessResult, PackError> {
        let mut lease = self.acquire(timeout_ms)?;
        // KEEP saturating: timeout_ms + grace caps at Duration::MAX without wrapping
        let recv_budget = Duration::from_millis(timeout_ms).saturating_add(self.reply_grace);
        let job = match dispatch(&mut lease.worker, job, recv_budget, timeout_ms) {
            Ok(outcome) => return outcome,
            Err(recovered_job) => recovered_job,
        };
        lease.worker = Some(ParseWorker::spawn()?);
        match dispatch(&mut lease.worker, job, recv_budget, timeout_ms) {
            Ok(outcome) => outcome,
            Err(_lost_job) => {
                lease.worker = None;
                tracing::error!(
                    operation = "parse_thread",
                    "newly spawned bounded parse worker disconnected before accepting its job",
                );
                Err(PackError::ParseFailed)
            }
        }
    }

    #[cfg(test)]
    pub(super) fn idle_thread_ids(&self) -> Vec<ThreadId> {
        self.lock_state().idle.iter().map(|w| w.thread_id).collect()
    }

    #[cfg(test)]
    pub(super) fn checked_out_count(&self) -> usize {
        self.lock_state().checked_out
    }
}

struct SlotLease<'a> {
    pool: &'a WorkerPool,
    worker: Option<ParseWorker>,
}

impl Drop for SlotLease<'_> {
    fn drop(&mut self) {
        let returned = self.worker.take();
        let mut guard = self.pool.lock_state();
        // KEEP saturating: checked_out was incremented once when this SlotLease was acquired
        guard.checked_out = guard.checked_out.saturating_sub(1);
        if let Some(worker) = returned {
            guard.idle.push(worker);
        }
        drop(guard);
        self.pool.available.notify_one();
    }
}

static WORKER_POOL: LazyLock<WorkerPool> = LazyLock::new(WorkerPool::new);

/// Runs `process(source, &config)` on a pooled persistent parse worker bounded
/// by `timeout_ms` and `recv_timeout`.
pub(super) fn process_bounded(
    source: &str,
    config: ProcessConfig,
    timeout_ms: u64,
) -> Result<ProcessResult, PackError> {
    let job = ParseJob {
        source: source.into(),
        config,
        #[cfg(test)]
        simulated_delay: None,
    };
    WORKER_POOL.run_job(job, timeout_ms)
}

#[cfg(test)]
pub(super) fn run_job_on_pool(
    pool: &WorkerPool,
    source: &str,
    config: ProcessConfig,
    timeout_ms: u64,
    simulated_delay: Option<Duration>,
) -> Result<ProcessResult, PackError> {
    let job = ParseJob {
        source: source.into(),
        config,
        simulated_delay,
    };
    pool.run_job(job, timeout_ms)
}

/// Sends `job` to `slot` and waits up to `recv_budget` for its reply. Returns
/// `Err(job)` only when an idle worker had already closed its job channel before
/// starting `job`, allowing the caller to spawn a replacement once.
fn dispatch(
    slot: &mut Option<ParseWorker>,
    job: ParseJob,
    recv_budget: Duration,
    timeout_ms: u64,
) -> Result<Result<ProcessResult, PackError>, ParseJob> {
    let Some(worker) = slot.as_ref() else {
        return Err(job);
    };
    if let Err(send_err) = worker.job_tx.send(job) {
        *slot = None;
        return Err(send_err.0);
    }
    match worker.reply_rx.recv_timeout(recv_budget) {
        Ok(Ok(result)) => Ok(Ok(*result)),
        Ok(Err(WorkerFailure::Pack(err @ PackError::ParseTimeout { .. }))) => {
            *slot = None;
            Ok(Err(err))
        }
        Ok(Err(WorkerFailure::Pack(err))) => Ok(Err(err)),
        Err(RecvTimeoutError::Timeout) => {
            // Abandon the wedged worker slot so `SlotLease::drop` frees capacity
            // without joining the stuck thread.
            *slot = None;
            tracing::warn!(
                operation = "parse_thread",
                timeout_ms,
                "bounded parse worker did not reply within timeout; abandoning wedged worker slot",
            );
            Ok(Err(PackError::ParseTimeout { timeout_ms }))
        }
        Ok(Err(WorkerFailure::Panicked)) | Err(RecvTimeoutError::Disconnected) => {
            *slot = None;
            tracing::error!(
                operation = "parse_thread",
                "the language pack panicked while parsing; recording the parse as failed",
            );
            Ok(Err(PackError::ParseFailed))
        }
    }
}

#[cfg(test)]
#[path = "parse_worker_tests.rs"]
mod tests;
