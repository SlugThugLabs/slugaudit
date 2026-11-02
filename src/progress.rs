//! Bridge between the sync layer's natural progress points and the
//! MCP-aware server that turns them into `/notifications/progress`
//! notifications.
//!
//! The sync layer must not depend on MCP per the layering rules; it has,
//! however, natural
//! progress points worth reporting to long-running tool calls
//! (per-file sampling during a publish, and the start/completion of
//! the `ensure_current` orchestration). [`ProgressSink`] is the
//! abstraction that lets the sync layer emit those events into
//! something the surrounding transport can render. The MCP-aware
//! implementation lives in `crate::server_progress` and translates each
//! [`ProgressEvent`] into a `notify_progress` call; tests and the
//! CLI path use [`NoopProgressSink`].
//!
//! Each variant carries a `phase: &'static str` so a stateful sink
//! can route events by stage without matching on payload shape.
//! `phase` is `'static` because the values are site-local string
//! literals in `publish` — keeping the enum a
//! single-screen summary.

/// A progress event raised by the sync layer.
///
/// `phase` is short, lowercase, and names the stage
/// (`ensuring_current`, `publishing`, `completed`). A consumer that wants
/// to display progress
/// percentages only needs to count [`ProgressEvent::Sampling`]
/// events scoped to the same phase name; intermediate
/// `Started`/`Completed` flags are for sinks that drive a UI
/// indicator rather than a percentage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    /// The work for `phase` has begun. Useful for sinks that show a
    /// spinner or progress bar from the moment `ensure_current`
    /// enters until it returns.
    Started {
        /// Lowercase name of the stage that started (for example
        /// `ensuring_current`).
        phase: &'static str,
    },
    /// Per-file progress during sampling. `current` is 1-based; the
    /// very first sample for a project emits `Sampling { current: 1,
    /// ... }`. `total` is the discovery-time count of files eligible
    /// for sampling; consumers fall back to "indeterminate" if it's
    /// zero.
    Sampling {
        /// Lowercase name of the stage being sampled (for example
        /// `publishing`).
        phase: &'static str,
        /// 1-based index of this sample within `total`.
        current: usize,
        /// Number of files eligible for sampling; zero means the total is
        /// unknown.
        total: usize,
    },
    /// The work for `phase` finished — emitted regardless of whether
    /// the underlying unit succeeded or failed, so a stateful sink
    /// can clear its UI on both outcomes.
    Completed {
        /// Lowercase name of the stage that finished.
        phase: &'static str,
    },
}

/// Send + Sync sink for progress events emitted by the sync layer.
///
/// Implementations may translate events into MCP
/// `/notifications/progress` notifications, log lines, a CLI status
/// bar, or simply drop them. [`NoopProgressSink`] is the default
/// implementation for tests and paths without a progress consumer.
pub trait ProgressSink: Send + Sync {
    /// Delivers one progress event to this sink.
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

/// `current / total` as a progress fraction, `0.0` when `total` is zero.
///
/// Returns `None`, after logging once, when a count exceeds `u32::MAX` (which
/// `f64` converts exactly): no real project has that many files, so the
/// notification is dropped rather than reporting a rounded fraction.
pub(crate) fn sampling_fraction(current: usize, total: usize) -> Option<f64> {
    match exact_fraction(current, total) {
        Ok(fraction) => Some(fraction),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not compute a progress fraction; notification skipped"
            );
            None
        }
    }
}

/// The fallible core of [`sampling_fraction`].
fn exact_fraction(current: usize, total: usize) -> Result<f64, std::num::TryFromIntError> {
    if total == 0 {
        return Ok(0.0);
    }
    Ok(f64::from(u32::try_from(current)?) / f64::from(u32::try_from(total)?))
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
