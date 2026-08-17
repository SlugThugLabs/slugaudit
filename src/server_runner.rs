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
use tokio::sync::Semaphore;
use tracing::Instrument;

/// Every tool handler does filesystem discovery, SQLite I/O, and (on first
/// sync) Tree-sitter parsing — all synchronous, potentially slow work that
/// would otherwise run directly on a Tokio worker thread and starve other
/// tasks scheduled on it. Bounds how many such blocking operations run at
/// once, independent of how many concurrent tool calls arrive.
pub(crate) const MAX_CONCURRENT_BLOCKING_OPS: usize = 8;

/// Runs `work` on Tokio's blocking thread pool rather than the calling
/// async task's worker thread, after acquiring a permit that caps how many
/// such operations run concurrently. `Semaphore::acquire` only errors if
/// the semaphore has been explicitly closed, which this server never does.
///
/// Wraps the whole call — permit wait, blocking work, and outcome — in one
/// span per tool call, logged to stderr only (see `main.rs`'s subscriber
/// setup): a start event, then completion/failure with elapsed time. Never
/// logs `work`'s actual arguments or return value — only the tool name and
/// timing/outcome, so SQL text, file content, and finding text never reach
/// the log. `work` itself may add its own fields to the current span (e.g.
/// revision id, row/edge counts) by calling `tracing::Span::current()`
/// from inside `tools::*` — the span is entered for the full duration of
/// the blocking closure via `blocking_span.enter()`, a sync guard, since
/// `spawn_blocking` runs on a thread the async instrumentation above does
/// not automatically follow.
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
    let span = tracing::info_span!("tool_call", tool = tool_name);
    let started = std::time::Instant::now();
    let blocking_span = span.clone();

    // Notify the consumer that the call has been received and is either
    // queued (waiting for a blocking permit) or about to start. Sending
    // this before the semaphore acquire is what makes a queued call
    // visible instead of silent.
    notify_progress(&progress, 0.0, format!("{tool_name} ensuring_current")).await;

    let result = async {
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
        notify_progress(&progress, 0.5, format!("{tool_name} publishing")).await;

        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _guard = blocking_span.enter();
            work()
        })
        .await
        .map_err(|error| ErrorData::internal_error(format!("tool task failed: {error}"), None))?
    }
    .instrument(span.clone())
    .await;

    // Final progress notification: the work is done and the response is
    // about to be returned. Progress reaches 1.0 whether the work succeeded
    // or failed — "completed" describes the operation's lifecycle, not its
    // outcome.
    notify_progress(&progress, 1.0, format!("{tool_name} completed")).await;

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let succeeded = result.is_ok();
    // Snapshot is taken inside `record_call` so the post-bump value is
    // the one logged here. The caller isn't observable, but the count
    // fields log accurately to the trace event so an operator reading
    // the log sees both the outcome of this call and the cumulative
    // rate at the time of the log line.
    let counters = crate::tools::ToolCounters::record_call(succeeded, elapsed_ms);
    let _guard = span.enter();
    match &result {
        Ok(_) => tracing::info!(
            elapsed_ms,
            call_count = counters.call_count,
            error_count = counters.error_count,
            "tool call completed",
        ),
        Err(error) => {
            tracing::warn!(
                elapsed_ms,
                error = %error.message,
                call_count = counters.call_count,
                error_count = counters.error_count,
                "tool call failed",
            );
        }
    }
    result
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
    if let Some((peer, token)) = progress {
        let _ = peer
            .notify_progress(
                ProgressNotificationParam::new(token.clone(), fraction)
                    .with_total(1.0)
                    .with_message(message),
            )
            .await;
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
}

impl ProgressSink for McpProgressSink {
    fn emit(&self, event: ProgressEvent) {
        let (fraction, message) = match event {
            ProgressEvent::Started { phase } => (0.0, format!("{phase}: started")),
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
                (fraction, format!("{phase}: {current}/{total}"))
            }
            ProgressEvent::Completed { phase } => (1.0, format!("{phase}: completed")),
        };

        let peer = self.peer.clone();
        let token = self.token.clone();
        // Per-event spawn is intentional: the future body is tiny (just
        // `notify_progress(...).await`), the per-call volume is small, and
        // avoiding a channel means the drain logic doesn't need its own
        // observable state. Errors here are silently dropped, matching
        // `run_blocking`'s "broken progress channel can never turn a
        // successful tool call into an error" stance.
        tokio::task::spawn(async move {
            notify_progress(&Some((peer, token)), fraction, message).await;
        });
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
        Some((peer, token)) => Arc::new(McpProgressSink { peer, token }),
        None => Arc::new(NoopProgressSink),
    }
}

#[cfg(test)]
#[path = "server_runner_tests.rs"]
mod tests;
