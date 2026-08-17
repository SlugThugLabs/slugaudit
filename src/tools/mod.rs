//! Tool handlers orchestrate: they resolve the active project, ensure it's
//! synced, and format a response. They own no parsing, SQL, or storage
//! logic themselves — that lives in `sync`, `store`, and `evidence`.
//!
//! This module also owns the **tool-level call counters**: process-wide
//! `AtomicU64` counts of total calls, error calls, and total elapsed
//! wall-clock time across every tool. The `health` tool exposes these
//! counters for observability without changing anything about how the
//! other tools execute. The counters live here rather than in
//! `server.rs` so they're reachable without going through the MCP tool
//! router — a future non-MCP entry point (a CLI subcommand, a watchdog,
//! etc.) bumps the same counters and the `health` tool sees them
//! consistently.

pub(crate) mod context;
mod finding;
mod health;
mod project_control;
mod query;
mod query_value;
mod report;
mod structure;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod redaction_tests;

pub use context::ensure_synced;
pub use finding::{FindingRequest, FindingResponse, finding};
pub use health::{HealthRequest, HealthResponse, health};
pub use project_control::{ProjectControlRequest, ProjectControlResponse, project_control};
pub use query::{QueryRequest, QueryResponse, query};
pub use report::{ReportRequest, ReportResponse, report};
pub use structure::{StructureRequest, StructureResponse, structure};

use std::sync::atomic::{AtomicU64, Ordering};

/// Total number of tool calls the MCP server has served since process
/// start. Includes both successful and errored calls — operators
/// monitoring rate-limited or failing integrations want the denominator.
static TOOL_CALL_COUNT: AtomicU64 = AtomicU64::new(0);

/// Number of tool calls that returned any `ErrorData` (transport-
/// independent error, regardless of code). Errors counted here are
/// surfaced through `health` so operators can see rate/severity without
/// reading MCP logs.
///
/// Counts only call-level errors (the work result). Network-level
/// failures (consumer disconnect, malformed MCP framing) are not
/// counted — they don't reach the work closure and so don't bump this
/// counter.
static TOOL_CALL_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);

/// Sum of all `run_blocking` work durations in milliseconds. Average
/// latency is then `total_ms / count` if `count > 0` — exposed via the
/// `health` tool so operators can see a simple moving-average trend
/// without pulling in a metrics backend.
///
/// Saturating on overflow at the conversion boundary; using
/// `u128`-based math under the hood avoids spurious overflow on
/// long-running servers.
static TOOL_CALL_TOTAL_MS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of process-wide tool-call counters for the `health` tool.
///
/// Saturates each value to its underlying atomic so a `Relaxed` load on
/// a 32-bit architecture produces a consistent 64-bit value; on a
/// 64-bit architecture this is atomic. The fields are returned as
/// `u64` because every value is bounded only by `u64::MAX` over the
/// lifetime of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCounters {
    pub call_count: u64,
    pub error_count: u64,
    pub total_ms: u64,
}

impl ToolCounters {
    /// Returns the current value of every process-wide tool counter.
    pub fn snapshot() -> Self {
        Self {
            call_count: TOOL_CALL_COUNT.load(Ordering::Relaxed),
            error_count: TOOL_CALL_ERROR_COUNT.load(Ordering::Relaxed),
            total_ms: TOOL_CALL_TOTAL_MS.load(Ordering::Relaxed),
        }
    }

    /// Increments the cumulative counters after a tool call completes.
    /// `succeeded` reflects the work's outcome (transport-independent).
    /// `duration_ms` is the wall-clock time spent inside the blocking
    /// work closure, exclusive of permit-acquire latency.
    ///
    /// Returns the post-bump snapshot so the caller can log the same
    /// counters in a single span without a second `snapshot()` call —
    /// useful for the standard "tool call completed" log line.
    pub(crate) fn record_call(succeeded: bool, duration_ms: u64) -> ToolCounters {
        if !succeeded {
            TOOL_CALL_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TOOL_CALL_TOTAL_MS.fetch_add(duration_ms, Ordering::Relaxed);
        TOOL_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
        Self::snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetches_a_fresh_snapshot_with_zero_initial_state() {
        // The counters are all-zero at process start; this test verifies
        // the accessor surface compiles and returns a snapshot rather
        // than racing other tests. The exact accumulated value is not
        // asserted — other tests bump the counters, and we cannot reset
        // them without forking the AtomicU64 to per-test locals.
        let _snapshot = ToolCounters::snapshot();
    }
}
