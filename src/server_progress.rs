//! The MCP progress plumbing of a tool call: the caller's progress target,
//! the single FIFO drain task that delivers monotonic `/notifications/progress`
//! updates for one call, the sink the sync layer emits [`ProgressEvent`]s into,
//! and the throttle on per-file `Sampling` notifications.

use crate::progress::{NoopProgressSink, ProgressEvent, ProgressSink};
use rmcp::model::{ProgressNotificationParam, ProgressToken, RequestMetaObject};
use rmcp::{Peer, RoleServer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// Caller peer + progress token pair for an in-flight MCP request.
pub(crate) type ProgressTarget = (Peer<RoleServer>, ProgressToken);

/// Upper bound for pre-tool sync progress so `1.0` is emitted exclusively when
/// [`ProgressSession::finish`] runs after the tool's own read or scan finishes.
pub(crate) const SYNC_PROGRESS_CAP: f64 = 0.9;

/// Sends one MCP `/notifications/progress` notification when the caller
/// requested progress, and nothing otherwise. Best-effort: a send failure is
/// logged as a warning, never returned, so a broken progress channel can
/// never turn a successful tool call into an error.
pub(crate) async fn notify_progress(
    progress: Option<&ProgressTarget>,
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
) -> Option<ProgressTarget> {
    meta.get_progress_token().map(|token| (peer.clone(), token))
}

/// One queued progress command for the per-call sequential drain task.
enum ProgressCommand {
    /// Send a progress notification whose fraction is clamped monotonically.
    Emit { fraction: f64, message: String },
    /// Stop draining queued events so the caller can await task completion.
    Close,
}

/// Shared completion-gated handle to the per-call progress FIFO queue.
#[derive(Clone)]
struct ProgressSender {
    sender: mpsc::UnboundedSender<ProgressCommand>,
    completed: Arc<AtomicBool>,
}

impl ProgressSender {
    fn emit(&self, fraction: f64, message: String) {
        if !self.completed.load(Ordering::Acquire)
            && let Err(error) = self
                .sender
                .send(ProgressCommand::Emit { fraction, message })
        {
            tracing::warn!(error = %error, "progress drain channel closed; notification dropped");
        }
    }

    fn close(&self, warn_on_closed: bool) {
        self.completed.store(true, Ordering::Release);
        match (self.sender.send(ProgressCommand::Close), warn_on_closed) {
            (Err(error), true) => {
                tracing::warn!(error = %error, "progress drain channel closed before completion");
            }
            (Err(error), false) => {
                tracing::debug!(error = %error, "progress drain channel already closed on drop");
            }
            (Ok(()), _) => {}
        }
    }
}

/// Per-call MCP progress session backed by a single FIFO channel and one
/// background drain task.
///
/// Every lifecycle and sync-layer notification for a tool call goes through
/// one `mpsc` queue drained sequentially by one task, which clamps each
/// outgoing fraction to `[max_sent_so_far, SYNC_PROGRESS_CAP]`. Closing and
/// awaiting the session before sending the final `1.0 "{tool} completed"`
/// notification guarantees strictly in-order, strictly monotonic delivery
/// with zero premature `1.0` frames and zero late stragglers.
pub(crate) struct ProgressSession {
    queue: ProgressSender,
    target: ProgressTarget,
    drain_task: Option<tokio::task::JoinHandle<()>>,
}

impl ProgressSession {
    /// Queues one progress update if the call has not yet completed.
    pub(crate) fn send(&self, fraction: f64, message: String) {
        self.queue.emit(fraction, message);
    }

    /// Gates any late inner emissions, flushes and awaits the drain task, and
    /// delivers the final `1.0 "{tool_name} completed"` notification.
    pub(crate) async fn finish(mut self, tool_name: &str) {
        self.queue.close(true);
        if let Some(handle) = self.drain_task.take()
            && let Err(error) = handle.await
        {
            tracing::warn!(error = %error, "progress drain task failed");
        }
        notify_progress(Some(&self.target), 1.0, format!("{tool_name} completed")).await;
    }
}

impl Drop for ProgressSession {
    fn drop(&mut self) {
        if self.drain_task.is_some() {
            self.queue.close(false);
        }
    }
}

/// Adapter that translates sync-layer [`ProgressEvent`]s into ordered updates
/// on the per-call [`ProgressSession`] channel.
pub(crate) struct McpProgressSink {
    queue: ProgressSender,
    /// Throttle for per-file `Sampling` notifications: at most one goes out
    /// per window, so a 60 k-file import does not flood the MCP host with
    /// notifications faster than ~10/s. `Started`/`Completed` always send.
    sampling_throttle: Throttle,
}

/// Coalesces events that arrive less than `window` apart: the first event
/// passes, the rest are dropped until the window elapses.
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
    fn should_emit(&self, now: std::time::Instant) -> Result<bool, crate::clock::ClockError> {
        // KEEP saturating: an instant not after the origin sits at offset zero, the first slot; the offset only orders events for coalescing
        let since_origin = now.saturating_duration_since(self.origin);
        let at = crate::clock::duration_millis(
            since_origin
                .checked_add(std::time::Duration::from_millis(1))
                .ok_or(crate::clock::ClockError::DurationOverflow)?,
        )?;
        let window_ms = crate::clock::duration_millis(self.window)?;
        let last = self.last.load(Ordering::SeqCst);
        if last != 0 {
            let inside_window = match at.checked_sub(last) {
                Some(gap) => gap < window_ms,
                None => window_ms > 0,
            };
            if inside_window {
                return Ok(false);
            }
        }
        Ok(self
            .last
            .compare_exchange(last, at, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok())
    }
}

impl ProgressSink for McpProgressSink {
    fn emit(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::Started { phase } => self.queue.emit(0.0, format!("{phase}: started")),
            ProgressEvent::Sampling {
                phase,
                current,
                total,
            } => {
                let Some(fraction) = crate::progress::sampling_fraction(current, total) else {
                    return;
                };
                let scaled = (fraction * SYNC_PROGRESS_CAP).min(SYNC_PROGRESS_CAP);
                match self
                    .sampling_throttle
                    .should_emit(std::time::Instant::now())
                {
                    Ok(true) => self
                        .queue
                        .emit(scaled, format!("{phase}: {current}/{total}")),
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(error = %error, "progress throttle clock unusable; sampling update dropped");
                    }
                }
            }
            ProgressEvent::Completed { phase } => {
                self.queue
                    .emit(SYNC_PROGRESS_CAP, format!("{phase}: completed"));
            }
        }
    }
}

/// Opens the per-call progress session and builds the inner [`ProgressSink`]
/// passed into the blocking closure, encapsulating the completion gate inside
/// the session. When `progress` is `None`, allocates no channel and spawns no
/// task, returning `(None, NoopProgressSink)`.
pub(crate) fn open_progress_session(
    progress: Option<ProgressTarget>,
) -> (Option<ProgressSession>, Arc<dyn ProgressSink>) {
    let Some((peer, token)) = progress else {
        return (None, Arc::new(NoopProgressSink));
    };
    let completed = Arc::new(AtomicBool::new(false));
    let (sender, mut receiver) = mpsc::unbounded_channel::<ProgressCommand>();
    let drain_target = (peer.clone(), token.clone());
    let drain_task = tokio::runtime::Handle::current().spawn(async move {
        let mut max_sent = 0.0_f64;
        while let Some(ProgressCommand::Emit { fraction, message }) = receiver.recv().await {
            if fraction.is_finite() {
                max_sent = fraction.clamp(max_sent, SYNC_PROGRESS_CAP);
                notify_progress(Some(&drain_target), max_sent, message).await;
            }
        }
    });
    let queue = ProgressSender { sender, completed };
    let sink = Arc::new(McpProgressSink {
        queue: queue.clone(),
        sampling_throttle: Throttle::new(std::time::Duration::from_millis(100)),
    });
    let session = ProgressSession {
        queue,
        target: (peer, token),
        drain_task: Some(drain_task),
    };
    (Some(session), sink)
}

#[cfg(test)]
#[path = "server_progress_tests.rs"]
mod tests;
