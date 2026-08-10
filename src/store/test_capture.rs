//! In-tree equivalent of `tracing_test::traced_test`'s `logs_contain` (minus
//! the macro magic that proved brittle for our local Rust-version target).
//! Lets a test exercise code that emits `tracing::warn!` / `info!` events and
//! assert on the formatted text without depending on `tracing-test`'s
//! procedural attribute.
//!
//! Shared by `connection::tests` and `netfs::tests`. `#[cfg(test)]`-only so
//! nothing in this file is reachable from production code paths.

use std::io;
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing_subscriber::fmt::MakeWriter;

/// A `MakeWriter` whose every per-event clone shares the same `Vec<u8>`
/// via an `Arc<Mutex<_>>`. Per-event writers are short-lived (one per
/// emitted event), but the underlying buffer is shared so the test sees a
/// single contiguous log of everything emitted inside the closure.
#[derive(Clone, Default)]
pub(crate) struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuf {
    type Writer = SharedBuf;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Run `f` under a fresh tracing subscriber that captures every emitted
/// event at or above `min_level` to an in-memory buffer, then return
/// `(result, captured_text)`. The buffer is per-call (and per-thread, since
/// `tracing::subscriber::with_default` only takes effect for the current
/// thread), so concurrent test execution can never observe one another and
/// sequential tests cannot leak captures.
pub(crate) fn capture_at_level<F, R>(f: F, min_level: Level) -> (R, String)
where
    F: FnOnce() -> R,
{
    let buf = SharedBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_max_level(min_level)
        .without_time()
        .with_target(true)
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let logs = String::from_utf8_lossy(&buf.0.lock().expect("capture buffer poisoned")).to_string();
    (result, logs)
}

/// Convenience: capture only `WARN` and above. This is what every rejection-
/// audit test currently needs; reach for `capture_at_level(..., INFO)` if a
/// future regression test wants to assert that an `info!` site fired.
pub(crate) fn capture_warns<F, R>(f: F) -> (R, String)
where
    F: FnOnce() -> R,
{
    capture_at_level(f, Level::WARN)
}
