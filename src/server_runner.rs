//! Execution mechanics for the MCP server: how a tool call actually runs.
//!
//! [`run_blocking`] moves each tool's synchronous work onto Tokio's
//! blocking thread pool, bounds how many such operations run at once
//! with a shared semaphore, and reports progress + timing for every
//! call. The MCP progress plumbing ([`McpProgressSink`],
//! [`build_inner_sink`], [`progress_target`]) lives here too, so the
//! tool-contract surface in `server.rs` stays a thin, declarative list
//! of handlers. Splitting these two concerns (what the tools are vs.
//! how they execute) keeps each file under the source-size cap without
//! hiding logic: the runner is one cohesive worker-pool contract, and
//! the progress adapter is its only MCP-aware dependency.

use crate::progress::{NoopProgressSink, ProgressEvent, ProgressSink};
use rmcp::model::{ProgressNotificationParam, ProgressToken, RequestMetaObject};
use rmcp::{ErrorData, Peer, RoleServer};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Semaphore;
use tracing::Instrument;

/// Every tool handler does filesystem discovery, SQLite I/O, and (on first
/// sync) Tree-sitter parsing — all synchronous, potentially slow work that
/// would otherwise run directly on a Tokio worker thread and starve other
/// tasks scheduled on it. Bounds how many such blocking operations run at
/// once, independent of how many concurrent tool calls arrive.
pub(crate) const MAX_CONCURRENT_BLOCKING_OPS: usize = 8;

/// Process-local monotonic id assigned to every tool call, so two
/// consecutive calls to the same tool are distinguishable in the log —
/// otherwise a session's call sequence is unreadable (`tool="query"`
/// repeated N times tells an operator nothing about chain or repetition).
///
/// Process-local is sufficient: `util::SESSION_ID` already identifies the
/// process, and the pair (session id, call id) is unique for the life of
/// that process. Never reset, never persisted, never sent to the caller.
static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);

/// Allocates the next call id. `Relaxed` suffices: the only contract is
/// that concurrent callers never receive the same value, which
/// `fetch_add` guarantees without ordering any other memory.
fn next_call_id() -> u64 {
    NEXT_CALL_ID.fetch_add(1, Ordering::Relaxed)
}

/// Runs `work` on Tokio's blocking thread pool rather than the calling
/// async task's worker thread, after acquiring a permit that caps how many
/// such operations run concurrently. `Semaphore::acquire` only errors if
/// the semaphore has been explicitly closed, which this server never does.
///
/// Wraps the whole call — permit wait, blocking work, and outcome — in one
/// span per tool call, logged to stderr only (see `main.rs`'s subscriber
/// setup): a start event, then exactly one completion event with elapsed
/// time and outcome. Every line this call emits carries the span's
/// `call_id` (unique per call, monotonic within the process) and
/// `session_id` (`util::SESSION_ID`), so a session's call sequence can be
/// recovered from the log by grepping one session id. The completion
/// event additionally carries the process-wide `ToolCounters` deltas
/// observed across the call and `outcome = "success" | "error"`; the
/// counters cannot be reset and concurrent calls bump them too, so they
/// are logged as deltas rather than as this call's exclusive share.
/// Never logs `work`'s actual arguments or return value — only the tool
/// name and timing/outcome, so SQL text, file content, and finding text
/// never reach the log. `work` itself may add its own fields to the
/// current span (e.g. revision id, row/edge counts) by calling
/// `tracing::Span::current()` from inside `tools::*` — the span is entered
/// for the full duration of the blocking closure via
/// `blocking_span.enter()`, a sync guard, since `spawn_blocking` runs on a
/// thread the async instrumentation above does not automatically follow.
///
/// When `progress` is `Some`, sends MCP progress notifications at three
/// points: "queued" before waiting on the semaphore, "working" once the
/// permit is acquired, and "completed" when the blocking work finishes
/// (whether it succeeded or failed). This lets the MCP consumer distinguish
/// a call that is queued behind the semaphore bound from one that is
/// actively running, rather than appearing frozen. The notifications are
/// best-effort: send failures are silently ignored so a broken progress
/// channel can never turn a successful tool call into an error.
pub(crate) async fn run_blocking<T: Send + 'static>(
    semaphore: Arc<Semaphore>,
    tool_name: &'static str,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
    progress: Option<(Peer<RoleServer>, ProgressToken)>,
) -> Result<T, ErrorData> {
    // `call_id` and `session_id` live on the span (not only on the
    // completion line) so the start line and every line a tool adds to the
    // current span carry the same pair. Metadata only: the tool's name, an
    // id, and the session — never its arguments or result.
    let span = tracing::info_span!(
        "tool_call",
        tool = tool_name,
        call_id = next_call_id(),
        session_id = %crate::util::session_id(),
    );
    let counters_before = crate::tools::ToolCounters::snapshot();
    let started = std::time::Instant::now();
    let blocking_span = span.clone();

    // Notify the consumer that the call has been received and is either
    // queued (waiting for a blocking permit) or about to start. Sending
    // this before the semaphore acquire is what makes a queued call
    // visible instead of silent.
    notify_progress(&progress, 0.0, format!("{tool_name} ensuring_current")).await;

    let result = acquire_and_run(semaphore, tool_name, work, &progress, blocking_span)
        .instrument(span.clone())
        .await;

    // Final progress notification: the work is done and the response is
    // about to be returned. Progress reaches 1.0 whether the work succeeded
    // or failed — "completed" describes the operation's lifecycle, not its
    // outcome.
    notify_progress(&progress, 1.0, format!("{tool_name} completed")).await;

    log_completion(&span, counters_before, started, result.as_ref().err());
    result
}

/// Waits for a permit, then runs `work` on a blocking thread with the
/// permit and the tool-call span moved into the closure.
async fn acquire_and_run<T: Send + 'static>(
    semaphore: Arc<Semaphore>,
    tool_name: &str,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
    progress: &Option<(Peer<RoleServer>, ProgressToken)>,
    blocking_span: tracing::Span,
) -> Result<T, ErrorData> {
    tracing::info!("tool call started");
    // `acquire_owned` returns an `OwnedSemaphorePermit` that holds an
    // `Arc` clone of the semaphore, so the permit is `'static` and can
    // be moved into `spawn_blocking`. This matters for cancellation:
    // if the outer tool-call future is cancelled, the permit stays
    // held inside the (still-running) blocking task instead of being
    // returned to the pool prematurely — keeping the semaphore's bound
    // honest for the full duration of the work.
    let permit = semaphore
        .acquire_owned()
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    // Now that we hold a permit, the call is actively running on a
    // blocking thread; per-file emissions from the tool-specific
    // progress sink overwrite this with the actual i/N ratio.
    notify_progress(progress, 0.5, format!("{tool_name} publishing")).await;

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _guard = blocking_span.enter();
        work()
    })
    .await
    .map_err(|error| ErrorData::internal_error(format!("tool task failed: {error}"), None))?
}

/// One completion line per call, both outcomes: a failure keeps WARN (and
/// keeps the error message), a success stays INFO, and `outcome` makes the
/// single marker greppable for the sequence log.
fn log_completion(
    span: &tracing::Span,
    counters_before: crate::tools::ToolCounters,
    started: std::time::Instant,
    error: Option<&ErrorData>,
) {
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let succeeded = error.is_none();
    // `record_call` bumps the process-wide counters and returns the
    // post-bump snapshot; `counters_before` was taken before the work
    // started. The difference is the movement observed across this call's
    // lifetime — concurrent calls' bumps are included, which is why it is
    // logged as a delta and never as an absolute or as this call's share.
    let counters_after = crate::tools::ToolCounters::record_call(succeeded, elapsed_ms);
    let calls_delta = counters_after
        .call_count
        .saturating_sub(counters_before.call_count);
    let errors_delta = counters_after
        .error_count
        .saturating_sub(counters_before.error_count);
    let outcome = if succeeded { "success" } else { "error" };
    let _guard = span.enter();
    match error {
        None => tracing::info!(
            elapsed_ms,
            outcome,
            calls_delta,
            errors_delta,
            "tool call completed",
        ),
        Some(error) => tracing::warn!(
            elapsed_ms,
            outcome,
            error = %error.message,
            calls_delta,
            errors_delta,
            "tool call completed",
        ),
    }
}

/// Sends one MCP `/notifications/progress` notification when the caller
/// requested progress, and nothing otherwise. Best-effort: send failures
/// are silently ignored so a broken progress channel can never turn a
/// successful tool call into an error. Shared by [`run_blocking`]'s three
/// lifecycle notifications and [`McpProgressSink`], so the notification
/// shape and the silent-drop contract live in one place.
async fn notify_progress(
    progress: &Option<(Peer<RoleServer>, ProgressToken)>,
    fraction: f64,
    message: String,
) {
    if let Some((peer, token)) = progress
        && let Err(error) = peer
            .notify_progress(
                ProgressNotificationParam::new(token.clone(), fraction)
                    .with_total(1.0)
                    .with_message(message),
            )
            .await
    {
        tracing::warn!(error = %error, "could not send tool progress notification");
    }
}

/// Extracts the caller's progress token from the MCP request metadata, if
/// it asked for progress notifications. Returns `None` when the caller did
/// not request progress, in which case the sync layer gets a
/// [`NoopProgressSink`] and emits nothing.
pub(crate) fn progress_target(
    meta: RequestMetaObject,
    peer: &Peer<RoleServer>,
) -> Option<(Peer<RoleServer>, ProgressToken)> {
    meta.get_progress_token().map(|token| (peer.clone(), token))
}

/// Adapter that translates sync-layer [`ProgressEvent`]s into MCP
/// `/notifications/progress` notifications. Held as `Arc<dyn
/// ProgressSink>` and cloned into a tool call's blocking closure;
/// `emit` spawns a one-shot task per event because `notify_progress`
/// returns a future that needs an executor to run, and the work being
/// instrumented runs on a `spawn_blocking` thread which can't await. The
/// per-event spawn is cheap (small body, only `notify_progress(...).await`),
/// so the volume of one Started + N Sampling + one Completed per long
/// call is well within the budget.
///
/// `Peer<RoleServer>` is an Arc-like reference that's cheap to clone, so
/// per-event spawning also avoids needing a shared channel + drain task —
/// keeping the implementation local and testable in isolation.
pub(crate) struct McpProgressSink {
    peer: Peer<RoleServer>,
    token: ProgressToken,
    /// Throttle for per-file `Sampling` notifications: at most one goes
    /// out per window, so a 60 k-file import doesn't spawn 60 k tasks and
    /// flood the MCP host with notifications the consumer can't use faster
    /// than ~10/s anyway. `Started`/`Completed` always send.
    sampling_throttle: std::sync::Mutex<Throttle>,
}

/// Coalesces events that arrive less than `window` apart: the first event
/// passes, the rest are dropped until the window elapses. Pure and
/// clock-injected so it is unit-testable without an MCP peer.
#[derive(Debug, Clone, Copy)]
struct Throttle {
    window: std::time::Duration,
    last: Option<std::time::Instant>,
}

impl Throttle {
    fn new(window: std::time::Duration) -> Self {
        Self { window, last: None }
    }

    fn should_emit(&mut self, now: std::time::Instant) -> bool {
        let Some(last) = self.last else {
            self.last = Some(now);
            return true;
        };
        if now.duration_since(last) < self.window {
            false
        } else {
            self.last = Some(now);
            true
        }
    }
}

impl McpProgressSink {
    fn spawn_notify(&self, fraction: f64, message: String) {
        let peer = self.peer.clone();
        let token = self.token.clone();
        // Per-event spawn is intentional: the future body is tiny (just
        // `notify_progress(...).await`), the throttled volume is small, and
        // avoiding a channel means the drain logic doesn't need its own
        // observable state. Errors here are silently dropped, matching
        // `run_blocking`'s "broken progress channel can never turn a
        // successful tool call into an error" stance.
        tokio::task::spawn(async move {
            notify_progress(&Some((peer, token)), fraction, message).await;
        });
    }
}

impl ProgressSink for McpProgressSink {
    fn emit(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::Started { phase } => {
                self.spawn_notify(0.0, format!("{phase}: started"));
            }
            ProgressEvent::Sampling {
                phase,
                current,
                total,
            } => {
                let fraction = if total > 0 {
                    current as f64 / total as f64
                } else {
                    0.0
                };
                // The last Sampling of a burst is allowed to be dropped:
                // `Completed` (1.0) always sends, so a consumer never
                // stalls at a stale intermediate percentage.
                let mut throttle = crate::util::lock_or_recover(&self.sampling_throttle);
                if !throttle.should_emit(std::time::Instant::now()) {
                    return;
                }
                drop(throttle);
                self.spawn_notify(fraction, format!("{phase}: {current}/{total}"));
            }
            ProgressEvent::Completed { phase } => {
                self.spawn_notify(1.0, format!("{phase}: completed"));
            }
        }
    }
}

/// Builds the sink that a tool closure will use to emit progress events
/// from inside the sync layer. When the caller asked for progress
/// notifications, this returns an [`McpProgressSink`] bound to that
/// caller's peer/token; otherwise it returns a [`NoopProgressSink`] so
/// the sync layer can call `emit` unconditionally without branching.
pub(crate) fn build_inner_sink(
    progress: Option<(Peer<RoleServer>, ProgressToken)>,
) -> Arc<dyn ProgressSink> {
    match progress {
        Some((peer, token)) => Arc::new(McpProgressSink {
            peer,
            token,
            // 100 ms ≈ 10 notifications/second — enough to show a live
            // i/N ratio without flooding the host on a huge import.
            sampling_throttle: std::sync::Mutex::new(Throttle::new(
                std::time::Duration::from_millis(100),
            )),
        }),
        None => Arc::new(NoopProgressSink),
    }
}

#[cfg(test)]
#[path = "server_runner_tests.rs"]
mod tests;
