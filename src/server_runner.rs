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

use crate::server_progress::ProgressSession;
use rmcp::ErrorData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Semaphore;
use tracing::Instrument;

/// Bounds how many blocking tool operations run concurrently across the server.
pub(crate) use crate::util::MAX_CONCURRENT_BLOCKING_OPS;

/// Process-local monotonic id assigned to every tool call so consecutive calls
/// to the same tool are distinguishable in the log.
static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);

fn next_call_id() -> u64 {
    NEXT_CALL_ID.fetch_add(1, Ordering::Relaxed)
}

/// Runs `work` on Tokio's blocking thread pool after acquiring a permit from
/// `semaphore`, logging one start and one completion event under a per-call
/// tracing span and flushing any [`ProgressSession`] before returning.
pub(crate) async fn run_blocking<T: Send + 'static>(
    semaphore: Arc<Semaphore>,
    tool_name: &'static str,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
    mut progress: Option<ProgressSession>,
) -> Result<T, ErrorData> {
    let span = tracing::info_span!(
        "tool_call",
        tool = tool_name,
        call_id = next_call_id(),
        session_id = %crate::util::session_id(),
    );
    let counters_before = crate::tools::ToolCounters::snapshot();
    let blocking_span = span.clone();

    if let Some(session) = &progress {
        session.send(0.0, format!("{tool_name} queued"));
    }

    let (result, elapsed_ms) =
        acquire_and_run(semaphore, tool_name, work, progress.as_ref(), blocking_span)
            .instrument(span.clone())
            .await;

    // Gate any late inner progress events, drain every queued event in FIFO
    // order, and emit the final `1.0 "{tool_name} completed"` notification.
    if let Some(session) = progress.take() {
        session.finish(tool_name).await;
    }

    log_completion(&span, counters_before, elapsed_ms, result.as_ref().err());
    result
}

/// Waits for a permit, then runs `work` on a blocking thread with the
/// permit and the tool-call span moved into the closure, returning the
/// work duration exclusive of permit-wait and progress-drain latency.
async fn acquire_and_run<T: Send + 'static>(
    semaphore: Arc<Semaphore>,
    tool_name: &str,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
    progress: Option<&ProgressSession>,
    blocking_span: tracing::Span,
) -> (Result<T, ErrorData>, Result<u64, crate::clock::ClockError>) {
    tracing::info!("tool call started");
    let permit = match semaphore.acquire_owned().await {
        Ok(permit) => permit,
        Err(error) => {
            return (
                Err(ErrorData::internal_error(error.to_string(), None)),
                Ok(0),
            );
        }
    };

    if let Some(session) = progress {
        session.send(0.0, format!("{tool_name} started"));
    }

    let started = std::time::Instant::now();
    let join_result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _guard = blocking_span.enter();
        work()
    })
    .await;
    let elapsed_ms = crate::clock::duration_millis(started.elapsed());
    let result = match join_result {
        Ok(outcome) => outcome,
        Err(error) => Err(ErrorData::internal_error(
            format!("tool task failed: {error}"),
            None,
        )),
    };
    (result, elapsed_ms)
}

/// Logs one completion line per call (`INFO` on success, `WARN` on error).
fn log_completion(
    span: &tracing::Span,
    counters_before: crate::tools::ToolCounters,
    elapsed_ms: Result<u64, crate::clock::ClockError>,
    error: Option<&ErrorData>,
) {
    let elapsed_ms = match elapsed_ms {
        Ok(elapsed_ms) => elapsed_ms,
        Err(error) => {
            tracing::error!(error = %error, "tool call duration unusable; completion not recorded");
            return;
        }
    };
    let counters_after = crate::tools::ToolCounters::record_call(error.is_none(), elapsed_ms);
    let (Some(calls_delta), Some(errors_delta)) = (
        counters_after
            .call_count
            .checked_sub(counters_before.call_count),
        counters_after
            .error_count
            .checked_sub(counters_before.error_count),
    ) else {
        tracing::error!("tool counters moved backwards; completion not logged");
        return;
    };
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

/// The machine-readable failure code from `ErrorData.data`, or `"none"`.
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
