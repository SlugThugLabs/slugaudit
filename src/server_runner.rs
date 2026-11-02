//! Execution mechanics for the MCP server: how a tool call actually runs.
//!
//! [`run_blocking`] moves each tool's synchronous work onto Tokio's
//! blocking thread pool, bounds how many such operations run at once
//! with a shared semaphore, and reports progress + timing for every
//! call. The MCP progress plumbing it sends through lives in
//! `server_progress`, so the tool-contract surface in `server.rs` stays a
//! thin, declarative list of handlers, the runner is one cohesive
//! worker-pool contract, and the progress adapter is its only MCP-aware
//! dependency.

use crate::server_progress::notify_progress;
use rmcp::model::ProgressToken;
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
/// name, timing/outcome, and on failure the error class, the
/// machine-readable code and the error message (see `log_completion`), so
/// SQL text, file content and finding text never reach the log. `work` itself may
/// add its own fields to the current span (e.g. revision id, row/edge
/// counts) by calling
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
/// best-effort: a send failure is logged, never returned, so a broken
/// progress channel can never turn a successful tool call into an error.
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
    let target = progress.as_ref();
    notify_progress(target, 0.0, format!("{tool_name} ensuring_current")).await;

    let result = acquire_and_run(semaphore, tool_name, work, target, blocking_span)
        .instrument(span.clone())
        .await;

    // Final progress notification: the work is done and the response is
    // about to be returned. Progress reaches 1.0 whether the work succeeded
    // or failed — "completed" describes the operation's lifecycle, not its
    // outcome.
    notify_progress(target, 1.0, format!("{tool_name} completed")).await;

    log_completion(&span, counters_before, started, result.as_ref().err());
    result
}

/// Waits for a permit, then runs `work` on a blocking thread with the
/// permit and the tool-call span moved into the closure.
async fn acquire_and_run<T: Send + 'static>(
    semaphore: Arc<Semaphore>,
    tool_name: &str,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
    progress: Option<&(Peer<RoleServer>, ProgressToken)>,
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

/// One completion line per call, both outcomes: a failure keeps WARN, a
/// success stays INFO, and `outcome` makes the single marker greppable for
/// the sequence log.
///
/// A failure line carries the JSON-RPC error class (`error_class`), the
/// closed-set machine code from `ErrorData.data` when the producer attached
/// one (`code`, `"none"` otherwise), the counters, and `error`, the
/// `ErrorData::message` (the cause the caller also receives). Error messages
/// are logged by design, once, here; the rmcp `response error` line is
/// capped by the default filter so a failure is never logged twice. The
/// redaction contract covers tool arguments, results and file content, not
/// error messages.
fn log_completion(
    span: &tracing::Span,
    counters_before: crate::tools::ToolCounters,
    started: std::time::Instant,
    error: Option<&ErrorData>,
) {
    let elapsed_ms = crate::clock::saturating_millis(started.elapsed());
    // `record_call` bumps the process-wide counters and returns the
    // post-bump snapshot; `counters_before` was taken before the work
    // started. The difference is the movement observed across this call's
    // lifetime — concurrent calls' bumps are included, which is why it is
    // logged as a delta and never as an absolute or as this call's share.
    let counters_after = crate::tools::ToolCounters::record_call(error.is_none(), elapsed_ms);
    let delta = |after: u64, before: u64| after.saturating_sub(before);
    let calls_delta = delta(counters_after.call_count, counters_before.call_count);
    let errors_delta = delta(counters_after.error_count, counters_before.error_count);
    let _guard = span.enter();
    match error {
        None => tracing::info!(
            elapsed_ms,
            outcome = "success",
            calls_delta,
            errors_delta,
            "tool call completed",
        ),
        Some(error) => tracing::warn!(
            elapsed_ms,
            outcome = "error",
            error_class = error.code.0,
            code = machine_code(error),
            error = %error.message,
            calls_delta,
            errors_delta,
            "tool call completed",
        ),
    }
}

/// The machine-readable failure code a producer attached to `ErrorData.data`
/// (`error_codes::payload`), or `"none"` when the producer attached none.
/// The set is closed and static, so the value is safe to use as a stable key.
fn machine_code(error: &ErrorData) -> &str {
    let Some(code) = error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(|code| code.as_str())
    else {
        return "none";
    };
    code
}

#[cfg(test)]
#[path = "server_runner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "server_runner_log_tests.rs"]
mod log_tests;
