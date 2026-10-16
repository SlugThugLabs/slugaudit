//! In-tree equivalent of `tracing_test::traced_test`'s `logs_contain` (minus
//! the macro magic that proved brittle for our local Rust-version target).
//! Lets a test exercise code that emits `tracing::warn!` / `info!` events and
//! assert on the formatted text without depending on `tracing-test`'s
//! procedural attribute.
//!
//! Shared by `connection::tests` and `netfs::tests`. `#[cfg(test)]`-only so
//! nothing in this file is reachable from production code paths.

use std::io;
use std::sync::{Arc, LazyLock, Mutex};
use tracing::Level;
use tracing::dispatcher::Dispatch;
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

/// A permissive, silent subscriber kept registered for the whole test
/// binary — it exists only to keep the *process-global* half of `tracing`'s
/// filtering state honest while captures come and go. It is never installed
/// as a thread's default, never writes anywhere, and is interested in every
/// callsite, so it can only ever enable, never disable.
///
/// Two pieces of tracing's filtering state are per process, not per
/// subscriber: each callsite's cached `Interest`, and the single max level
/// (`LevelFilter::current()`). Both are recomputed from the registered
/// dispatchers every time a dispatcher is registered. When the registry
/// holds exactly one dispatcher, `tracing_core` skips it and recomputes from
/// `dispatcher::get_default()` of whichever thread triggered the
/// recomputation — and a thread that is *about* to install a capture
/// subscriber still has no dispatcher, so `NoSubscriber` answers
/// `Interest::never()` for every callsite [`Dispatch::new`] touches. One
/// capture starting at the wrong moment therefore leaves callsites cached as
/// `never` process-wide, and `tracing`'s macros drop those events for every
/// *other* test's live capture too — including the `INFO tool call started`
/// lines the log-capture tests assert on, which is what made them flaky
/// under parallel load.
///
/// Holding a second dispatcher registered for the life of the binary keeps
/// the registry above one entry, so every later recomputation sees the
/// dispatchers that are actually live. [`capture_at_level`] forces this to
/// initialize before the capture's own subscriber is registered.
static PERMISSIVE_BACKSTOP: LazyLock<Dispatch> = LazyLock::new(|| {
    Dispatch::new(
        tracing_subscriber::fmt()
            .with_writer(io::sink)
            .with_max_level(Level::TRACE)
            .finish(),
    )
});

/// Run `f` under a fresh tracing subscriber that captures every emitted
/// event at or above `min_level` to an in-memory buffer, then return
/// `(result, captured_text)`. The buffer is per-call and the subscriber is
/// installed thread-locally (`tracing::subscriber::with_default`), so
/// concurrent test execution can never write into another test's buffer and
/// sequential tests cannot leak captures — provided the process-global
/// filter state described on [`PERMISSIVE_BACKSTOP`] stays permissive, which
/// is why that runs first.
pub(crate) fn capture_at_level<F, R>(f: F, min_level: Level) -> (R, String)
where
    F: FnOnce() -> R,
{
    LazyLock::force(&PERMISSIVE_BACKSTOP);
    let buf = SharedBuf::default();
    // `with_ansi(false)` is load-bearing, not cosmetic. The formatter colours
    // by default and emits escape sequences *between* a field's name, its
    // `=` and its value, so captured text reads
    // `tool\x1b[0m\x1b[2m=\x1b[0m"probe"` and every test that probes the
    // capture with `logs.contains("tool=\"probe\"")` silently matches
    // nothing. That made three `server_runner` tests pass only when the
    // ambient `NO_COLOR` happened to be set — green locally, red in CI.
    // `without_time` was added for the same reason; a new formatting default
    // must not be allowed to reintroduce the problem, which is why
    // `captured_text_carries_no_ansi_escapes` asserts it.
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_max_level(min_level)
        .without_time()
        .with_target(true)
        .with_ansi(false)
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
