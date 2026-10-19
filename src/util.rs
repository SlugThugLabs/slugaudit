// slugaudit-line-exception: approved-by=slugthug; reason=the shared bounded-read helper and its cap-boundary tests belong beside the existing helpers they extend; splitting them would scatter one cap contract across modules
//! Small shared helpers used across multiple modules, kept here to avoid
//! drifting duplicate implementations.
//!
//! This module is intentionally narrow: only helpers with no more specific
//! home belong here. If a new helper clearly belongs to an existing module
//! (e.g. a hashing utility belongs in `sync::hash`, a time utility belongs
//! wherever the majority of its callers live), put it there instead of
//! defaulting to `util`. This file should not become a dumping ground.

use std::io::{IsTerminal, Read as _};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime};

pub(crate) use crate::clock::mtime_unix_seconds;

static LAST_UNIX_TIME: AtomicI64 = AtomicI64::new(0);

/// Returns whether an externally supplied UTF-8 string exceeds its byte cap.
/// Tool request validation uses bytes because the transport and storage
/// budgets are byte-based, and this avoids allocating a copy of the input.
pub(crate) fn exceeds_input_bytes(value: &str, max_bytes: usize) -> bool {
    value.len() > max_bytes
}

/// Returns a stable, non-reversible identifier for a value that must be
/// correlated in logs without disclosing user-controlled data.
pub(crate) fn redacted_log_id(value: impl AsRef<[u8]>) -> String {
    blake3::hash(value.as_ref()).to_hex().to_string()
}

/// Returns a stable, non-reversible identifier for a filesystem path.
/// Paths can contain usernames, repository names, and other private data, so
/// they must not be emitted verbatim in server logs.
pub(crate) fn redacted_path(path: &std::path::Path) -> String {
    redacted_log_id(path.to_string_lossy().as_bytes())
}

/// Canonicalizes `path` for use as a stable identity key, falling back to
/// the path as given when canonicalization fails.
///
/// Watch-state keys and PATH comparisons depend on the canonical form. The
/// fallback is deliberate — the caller keeps working — but it must not be
/// silent: if canonicalization fails while a watcher registered under a
/// different spelling, the mismatch is invisible unless this says so.
#[must_use]
pub(crate) fn canonical_or_self(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|error| {
        tracing::warn!(
            path = %redacted_path(path),
            error = %error,
            "could not canonicalize; continuing with the path as given",
        );
        path.to_path_buf()
    })
}

/// Cap for foreign text files the CLI reads to edit them: other tools'
/// configs and the user's shell rc. None of them has a prior that they are
/// small, so reads are bounded like everything else in this crate.
pub(crate) const MAX_FOREIGN_TEXT_BYTES: u64 = 1024 * 1024;

/// Why a capped file read failed.
#[derive(Debug)]
pub(crate) enum CappedReadError {
    /// The file could not be inspected or read.
    Io(std::io::Error),
    /// The file exceeds the caller's cap. `observed` is the stat size when
    /// that was already over the cap, or the bounded-read length when the
    /// file grew in the stat/read window; it is never a truncation.
    TooLarge { limit: u64, observed: u64 },
}

impl std::fmt::Display for CappedReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::TooLarge { limit, observed } => {
                write!(formatter, "file is {observed} bytes, over the {limit}-byte cap")
            }
        }
    }
}

impl std::error::Error for CappedReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::TooLarge { .. } => None,
        }
    }
}

/// Reads a file's bytes, refusing to load more than `max_bytes`.
///
/// The metadata check rejects the common case without opening the file; the
/// read itself is bounded by `take(max_bytes + 1)`, so a file that grows past
/// the cap between the stat and the read cannot allocate an unbounded buffer.
/// The observed length is reported, never truncated.
pub(crate) fn read_file_capped_bytes(
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<Vec<u8>, CappedReadError> {
    let metadata = std::fs::metadata(path).map_err(CappedReadError::Io)?;
    if metadata.len() > max_bytes {
        return Err(CappedReadError::TooLarge {
            limit: max_bytes,
            observed: metadata.len(),
        });
    }
    let file = std::fs::File::open(path).map_err(CappedReadError::Io)?;
    let mut bytes = Vec::with_capacity(metadata.len().min(64 * 1024) as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(CappedReadError::Io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(CappedReadError::TooLarge {
            limit: max_bytes,
            observed: bytes.len() as u64,
        });
    }
    Ok(bytes)
}

/// Reads a UTF-8 file whose size must not exceed `max_bytes`.
///
/// Delegates the bound to [`read_file_capped_bytes`] so both the stat and
/// the read are capped; a file that grows in the window is reported, not
/// loaded. The returned error keeps `NotFound` and other I/O kinds intact
/// for callers that branch on them.
pub(crate) fn read_file_capped(path: &std::path::Path, max_bytes: u64) -> std::io::Result<String> {
    let bytes = read_file_capped_bytes(path, max_bytes).map_err(|error| match error {
        CappedReadError::Io(source) => source,
        too_large @ CappedReadError::TooLarge { .. } => {
            std::io::Error::new(std::io::ErrorKind::InvalidData, too_large.to_string())
        }
    })?;
    String::from_utf8(bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("file is not valid UTF-8: {error}"),
        )
    })
}

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
    const DIGITS: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(DIGITS[usize::from(byte >> 4)]);
        hex.push(DIGITS[usize::from(byte & 0x0f)]);
    }
    hex
}

/// The serialized JSON length of `value` in bytes, measured without
/// materializing the string. Both response-byte budgets (`query`'s rows,
/// `leads`' rows) are enforced against what the caller actually receives,
/// so they measure the serialized form rather than the in-memory one.
pub(crate) fn serialized_len<T: serde::Serialize>(value: &T) -> Result<usize, serde_json::Error> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(buf.len());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

/// The current Unix timestamp in seconds. A clock before the epoch (or past
/// the `i64` range) is a documented, deliberate recovery: it yields `0`
/// (respectively `i64::MAX`), logged with the source error, and the monotonic
/// guard below keeps stored timestamps from moving backwards.
pub(crate) fn now_unix() -> i64 {
    let observed = crate::clock::unix_seconds(SystemTime::now());
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
