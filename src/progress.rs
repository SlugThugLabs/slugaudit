//! Bridge between the sync layer's natural progress points and the
//! MCP-aware server that turns them into `/notifications/progress`
//! notifications.
//!
//! The sync layer must not depend on MCP per the layering rules
//! ([ARCHITECTURE.md](../ARCHITECTURE.md)); it has, however, natural
//! progress points worth reporting to long-running tool calls
//! (per-file sampling during a publish, and the start/completion of
//! the ensure_current orchestration). [`ProgressSink`] is the
//! abstraction that lets the sync layer emit those events into
//! something the surrounding transport can render. The MCP-aware
//! implementation lives in `crate::server_runner` and translates each
//! [`ProgressEvent`] into a `notify_progress` call; tests and the
//! CLI path use [`NoopProgressSink`].//!
//! Each variant carries a `phase: &'static str` so a stateful sink
//! can route events by stage without matching on payload shape.
//! `phase` is `'static` because the values are site-local string
//! literals in `publish` and `reconcile` — keeping the enum a
//! single-screen summary.

/// A progress event raised by the sync layer.
///
/// `phase` is short, lowercase, and matches the diagram annotations
/// in `ARCHITECTURE.md` (`ensuring_current`, `publishing`,
/// `completed`). A consumer that wants to display progress
/// percentages only needs to count [`ProgressEvent::Sampling`]
/// events scoped to the same phase name; intermediate
/// `Started`/`Completed` flags are for sinks that drive a UI
/// indicator rather than a percentage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    /// The work for `phase` has begun. Useful for sinks that show a
    /// spinner or progress bar from the moment `ensure_current`
    /// enters until it returns.
    Started { phase: &'static str },
    /// Per-file progress during sampling. `current` is 1-based; the
    /// very first sample for a project emits `Sampling { current: 1,
    /// ... }`. `total` is the discovery-time count of files eligible
    /// for sampling; consumers fall back to "indeterminate" if it's
    /// zero.
    Sampling {
        phase: &'static str,
        current: usize,
        total: usize,
    },
    /// The work for `phase` finished — emitted regardless of whether
    /// the underlying unit succeeded or failed, so a stateful sink
    /// can clear its UI on both outcomes.
    Completed { phase: &'static str },
}

/// Send + Sync sink for progress events emitted by the sync layer.
///
/// Implementations may translate events into MCP
/// `/notifications/progress` notifications, log lines, a CLI status
/// bar, or simply drop them. [`NoopProgressSink`] is the default
/// implementation for tests and paths without a progress consumer.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: ProgressEvent);
}

/// Default sink that discards every event. Cheap to construct and
/// trivially `Send + Sync` so it can be passed through the sync
/// layer wherever a real consumer isn't warranted (tests, one-shot
/// CLI commands, code paths that haven't yet been wired to a
/// progress channel).
#[derive(Default, Clone, Copy)]
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn emit(&self, _event: ProgressEvent) {}
}
