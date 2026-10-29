//! Small shared helpers used across multiple modules, kept here to avoid
//! drifting duplicate implementations.
//!
//! This module is intentionally narrow: only helpers with no more specific
//! home belong here. If a new helper clearly belongs to an existing module
//! (e.g. a hashing utility belongs in `sync::hash`, a time utility belongs
//! wherever the majority of its callers live), put it there instead of
//! defaulting to `util`. This file should not become a dumping ground.

mod capped_read;
mod style;
mod time;

use std::sync::Mutex;

pub(crate) use crate::clock::mtime_unix_seconds;
pub(crate) use capped_read::{
    CappedReadError, MAX_FOREIGN_TEXT_BYTES, read_file_capped, read_file_capped_bytes,
};
pub(crate) use style::Style;
pub(crate) use time::{BudgetBound, Deadline, at_least_timestamp, now_unix};

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
            path = %path.display(),
            error = %error,
            "could not canonicalize; continuing with the path as given",
        );
        path.to_path_buf()
    })
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
    let mut guard = lock_or_recover(&SESSION_ID, "session_id");
    *guard.get_or_insert_with(uuid::Uuid::new_v4)
}

/// Tests override the live session ID to simulate a fresh process boot
/// without spawning a new binary. Production code never calls this.
#[cfg(test)]
pub(crate) fn override_session_id_for_test(id: uuid::Uuid) {
    let mut guard = lock_or_recover(&SESSION_ID, "session_id");
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
///
/// `mutex_name` is the static label emitted with the recovery line. Without
/// it the operator sees that *a* mutex was poisoned but not which state was
/// recovered, which is the only actionable part of the message.
pub(crate) fn lock_or_recover<'a, T>(
    mutex: &'a Mutex<T>,
    mutex_name: &'static str,
) -> std::sync::MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::error!(
            operation = "lock_or_recover",
            mutex = mutex_name,
            "mutex was poisoned; recovering inner value",
        );
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

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
