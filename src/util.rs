//! Small shared helpers used across multiple modules, kept here to avoid
//! drifting duplicate implementations.
//!
//! This module is intentionally narrow: only helpers with no more specific
//! home belong here. If a new helper clearly belongs to an existing module
//! (e.g. a hashing utility belongs in `sync::hash`, a time utility belongs
//! wherever the majority of its callers live), put it there instead of
//! defaulting to `util`. This file should not become a dumping ground.

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static LAST_UNIX_TIME: AtomicI64 = AtomicI64::new(0);

/// Minimal ANSI styling for the human-facing CLI surface (`menu`, `help`).
///
/// Detects once, at construction, whether stdout is a real terminal. When
/// it is not (piped to a file, captured by a test harness, or fed into a
/// CI log), every style method returns its input unchanged, so no escape
/// sequences ever leak into redirected output or downstream tools. When
/// stdout IS a terminal, the methods wrap the text in the named ANSI
/// SGR code and reset afterwards.
///
/// `help`, `menu`, and `connect` are the only paths that build with these
/// — the stdio MCP server (`serve`) must never touch stdout for humans,
/// and the server's diagnostics on stderr already disable ANSI
/// unconditionally in `main`. This helper is deliberately the single
/// place ANSI codes are produced.
pub(crate) struct Style {
    enabled: bool,
}

impl Style {
    /// A `Style` that colors only if stdout is a terminal at call time.
    pub(crate) fn stdout() -> Self {
        Self {
            enabled: std::io::stdout().is_terminal(),
        }
    }

    /// A `Style` that never colors. Test-only: enables assertions that a
    /// rendering produces the exact plain text regardless of the
    /// destination terminal.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self { enabled: false }
    }

    #[cfg(test)]
    pub(crate) const fn colored() -> Self {
        Self { enabled: true }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if !self.enabled {
            return text.to_owned();
        }
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    pub(crate) fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    pub(crate) fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }

    pub(crate) fn cyan(&self, text: &str) -> String {
        self.wrap("36", text)
    }

    pub(crate) fn green(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    pub(crate) fn yellow(&self, text: &str) -> String {
        self.wrap("33", text)
    }
}

/// Serializes tests that mutate process-global environment variables
/// (`SLUGTHUG_HOME`, `HOME`) so parallel test threads can't observe each
/// other's mutations. Test-only; production code never touches it.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that depend on the process-global `SESSION_ID`.
/// Two kinds of tests race over that global: session-flipping tests that call
/// `override_session_id_for_test` to simulate a fresh process boot, and
/// read/write tests that query or store `findings` rows scoped by
/// `session_id()`. Test-only; production code never touches it.
#[cfg(test)]
pub(crate) static SESSION_TEST_LOCK: Mutex<()> = Mutex::new(());

static SESSION_ID: Mutex<Option<uuid::Uuid>> = Mutex::new(None);

/// Returns the current session UUID, allocating a fresh v4 on first
/// call. After the first call the inner `Option` is `Some`, so every
/// subsequent call is a single lock + clone and never regenerates.
#[must_use]
pub(crate) fn session_id() -> uuid::Uuid {
    let mut guard = lock_or_recover(&SESSION_ID);
    *guard.get_or_insert_with(uuid::Uuid::new_v4)
}

/// Tests override the live session ID to simulate a fresh process boot
/// without spawning a new binary. Production code never calls this.
#[cfg(test)]
pub(crate) fn override_session_id_for_test(id: uuid::Uuid) {
    let mut guard = lock_or_recover(&SESSION_ID);
    *guard = Some(id);
}

/// Acquires a mutex guard, recovering the inner value if the mutex was
/// poisoned by a previous holder panicking. Without this recovery, a
/// single panic inside any critical section guarded by a `Mutex` would
/// crash the entire process on the next event: `Mutex::lock()` returns a
/// `PoisonError`, and unwrapping it propagates the original panic out of
/// the calling thread.
///
/// Recovery is logged at `error` (poisoning is an unexpected, recoverable
/// condition we want visible) but never escalated — the inner data is
/// already maintained across panic boundaries inside the `MutexGuard`,
/// so a successor lock just continues the work. The trade-off: the
/// recovered data may be in a logically inconsistent state (a partially-
/// applied mutation that panicked mid-step), so callers that mutate
/// multiple fields atomically document and test that invariant.
pub(crate) fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::error!("mutex was poisoned; recovering inner value");
        poisoned.into_inner()
    })
}

/// Hex-encodes a byte slice as a lowercase hex string. Used by both the
/// hashing layer and the query value converter.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // `write!` into a String never fails.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// The current Unix timestamp in seconds. Returns 0 if the system clock is
/// before the epoch (defensive — not expected in practice).
pub(crate) fn now_unix() -> i64 {
    let observed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });
    let mut previous = LAST_UNIX_TIME.load(Ordering::Relaxed);
    loop {
        let next = observed.max(previous);
        match LAST_UNIX_TIME.compare_exchange_weak(
            previous,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

/// Returns a wall-clock timestamp that is at least the last persisted value.
/// SQLite row IDs remain the canonical ordering key; this only prevents the
/// human-readable timestamps from moving backwards across restarts.
pub(crate) fn at_least_timestamp(observed: i64, persisted: Option<i64>) -> i64 {
    observed.max(persisted.unwrap_or(0))
}

/// Unix-epoch seconds for a file's mtime, or `None` when the platform
/// cannot report one (or reports a pre-epoch time). Stored alongside the
/// content hash so the sync sweep can skip unchanged files with one stat
/// instead of a full read+hash.
pub(crate) fn mtime_unix_seconds(metadata: &std::fs::Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?;
    let elapsed = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(elapsed.as_secs()).ok()
}

/// A cooperative wall-clock deadline for one sync operation.
///
/// Created once at the entry of an operation that may iterate many times
/// (a publish spanning CAS retries, a barrier sync spanning reconcile
/// iterations) so the budget covers the whole operation rather than
/// resetting per phase — otherwise a pathological repo could restart the
/// clock on every retry and stall a tool call indefinitely.
///
/// `exceeded` is checked at cooperative points inside hot loops (per
/// discovered file, per dirty path, per barrier iteration). It never
/// interrupts anything; it only reports that the budget has been spent, so
/// the loop can fail closed with a `TimeBudgetExceeded` error instead of
/// looping forever.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    started: Instant,
    budget: Duration,
}

impl Deadline {
    pub(crate) fn after(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }

    /// Returns the elapsed time if the budget is exhausted, else `None`.
    /// Tests inject `Duration::from_nanos(1)` so any positive elapsed time
    /// trips the deadline deterministically without waiting out the
    /// production-sized budget.
    pub(crate) fn exceeded(&self) -> Option<Duration> {
        let elapsed = self.started.elapsed();
        (elapsed >= self.budget).then_some(elapsed)
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn a_zero_budget_is_exceeded_immediately() {
        let deadline = Deadline::after(Duration::ZERO);
        assert!(deadline.exceeded().is_some());
    }

    #[test]
    fn a_generous_budget_is_not_exceeded_immediately() {
        let deadline = Deadline::after(Duration::from_secs(60));
        assert!(deadline.exceeded().is_none());
    }
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
