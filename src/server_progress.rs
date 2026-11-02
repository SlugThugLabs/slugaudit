//! The MCP progress plumbing of a tool call: the caller's progress target,
//! the sink the sync layer emits [`ProgressEvent`]s into, the throttle on
//! per-file `Sampling` notifications, and the one function that sends a
//! `/notifications/progress` notification. `server_runner::run_blocking`
//! sends its lifecycle notifications through the same function.

use crate::progress::{NoopProgressSink, ProgressEvent, ProgressSink};
use rmcp::model::{ProgressNotificationParam, ProgressToken, RequestMetaObject};
use rmcp::{Peer, RoleServer};
use std::sync::Arc;

/// Sends one MCP `/notifications/progress` notification when the caller
/// requested progress, and nothing otherwise. Best-effort: send failures
/// are silently ignored so a broken progress channel can never turn a
/// successful tool call into an error. Shared by `run_blocking`'s three
/// lifecycle notifications and [`McpProgressSink`], so the notification
/// shape and the silent-drop contract live in one place.
pub(crate) async fn notify_progress(
    progress: Option<&(Peer<RoleServer>, ProgressToken)>,
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
    meta: &RequestMetaObject,
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
    sampling_throttle: Throttle,
    /// Runtime captured at construction (inside the tool call). `Sampling`
    /// events are emitted from plain `std::thread::scope` workers that have
    /// no runtime context, where `tokio::task::spawn` would panic.
    runtime: tokio::runtime::Handle,
}

/// Coalesces events that arrive less than `window` apart: the first event
/// passes, the rest are dropped until the window elapses. Lock-free, so no
/// panic elsewhere can poison it, and clock-injected so it is unit-testable
/// without an MCP peer. Times are whole milliseconds after `origin`.
#[derive(Debug)]
struct Throttle {
    window: std::time::Duration,
    origin: std::time::Instant,
    /// Milliseconds after `origin`, plus one, of the last event that passed;
    /// `0` until one has.
    last: std::sync::atomic::AtomicU64,
}

impl Throttle {
    fn new(window: std::time::Duration) -> Self {
        Self {
            window,
            origin: std::time::Instant::now(),
            last: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Whether an event at `now` passes. Of two threads racing for the same
    /// window, only the one whose update lands passes.
    fn should_emit(&self, now: std::time::Instant) -> bool {
        use std::sync::atomic::Ordering;
        let at = crate::clock::saturating_millis(now.saturating_duration_since(self.origin))
            .saturating_add(1);
        let last = self.last.load(Ordering::SeqCst);
        if last != 0 && at.saturating_sub(last) < crate::clock::saturating_millis(self.window) {
            return false;
        }
        self.last
            .compare_exchange(last, at, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
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
        self.runtime.spawn(async move {
            notify_progress(Some(&(peer, token)), fraction, message).await;
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
                let Some(fraction) = crate::progress::sampling_fraction(current, total) else {
                    return;
                };
                // The last Sampling of a burst is allowed to be dropped:
                // `Completed` (1.0) always sends, so a consumer never
                // stalls at a stale intermediate percentage.
                if !self
                    .sampling_throttle
                    .should_emit(std::time::Instant::now())
                {
                    return;
                }
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
            runtime: tokio::runtime::Handle::current(),
            // 100 ms ≈ 10 notifications/second — enough to show a live
            // i/N ratio without flooding the host on a huge import.
            sampling_throttle: Throttle::new(std::time::Duration::from_millis(100)),
        }),
        None => Arc::new(NoopProgressSink),
    }
}

#[cfg(test)]
#[path = "server_progress_tests.rs"]
mod tests;
