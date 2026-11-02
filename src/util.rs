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

pub use crate::clock::ClockError;
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

/// Serializes tests that mutate process-global environment variables
/// (`SLUGTHUG_HOME`, `HOME`) so parallel test threads can't observe each
/// other's mutations. Test-only; production code never touches it.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Joins a test thread. A panic in the thread comes back as its message, so a
/// test can report it with `.check(..)` instead of unwrapping the join error.
/// Test-only.
#[cfg(test)]
pub(crate) fn join_thread<T>(handle: std::thread::JoinHandle<T>) -> Result<T, String> {
    handle.join().map_err(|payload| {
        if let Some(message) = payload.downcast_ref::<&str>() {
            (*message).to_owned()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            "the thread panicked with a non-string payload".to_owned()
        }
    })
}

static SESSION_ID: std::sync::OnceLock<uuid::Uuid> = std::sync::OnceLock::new();

/// Returns the current session UUID, allocating a fresh v4 on first
/// call; every later call returns the same value and never regenerates.
#[must_use]
pub(crate) fn session_id() -> uuid::Uuid {
    *SESSION_ID.get_or_init(uuid::Uuid::new_v4)
}

/// A mutex whose earlier holder panicked inside its critical section, so the
/// data it guards may be half-updated.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("mutex {mutex} was poisoned by a panic in an earlier holder")]
pub struct LockPoisoned {
    /// The static label of the poisoned mutex.
    mutex: &'static str,
}

/// A poisoned mutex stays poisoned for the life of the process, so the tool
/// error says the server must be restarted.
impl From<LockPoisoned> for rmcp::ErrorData {
    fn from(error: LockPoisoned) -> Self {
        Self::internal_error(format!("{error}; restart the SlugAudit server"), None)
    }
}

/// Acquires a mutex guard. A mutex poisoned by a panicking holder is an
/// error the operation returns: the data it guards may be half-updated, so
/// it is never read. `mutex_name` is the static label the error carries, so
/// the failure names which state was lost.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when an earlier holder panicked.
pub(crate) fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    mutex_name: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, LockPoisoned> {
    mutex
        .lock()
        .map_err(|_poisoned| LockPoisoned { mutex: mutex_name })
}

/// Hex-encodes a byte slice as a lowercase hex string. Used by both the
/// hashing layer and the query value converter.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(hex_digit(byte >> 4));
        hex.push(hex_digit(byte & 0x0f));
    }
    hex
}

/// The lowercase hex digit of the low four bits of `nibble`.
fn hex_digit(nibble: u8) -> char {
    match nibble & 0x0f {
        digit @ 0..=9 => char::from(b'0' + digit),
        letter => char::from(b'a' + (letter - 10)),
    }
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
