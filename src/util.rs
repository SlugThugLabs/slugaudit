//! Small shared helpers used across multiple modules, kept here to avoid
//! drifting duplicate implementations.

mod atomic_write;
mod capped_read;
mod proc;
mod style;
#[cfg(test)]
pub(crate) mod test_capture;
mod time;

use std::sync::Mutex;

pub(crate) use crate::clock::ClockError;
pub(crate) use crate::clock::mtime_unix_seconds;
pub(crate) use atomic_write::{same_open_file, sync_directory, write_atomic_chunks_with};
pub(crate) use capped_read::{
    CappedReadError, MAX_FOREIGN_TEXT_BYTES, open_regular_file_nonblocking, read_file_capped,
    read_file_capped_bytes,
};
pub(crate) use proc::{BoundedProcError, run_bounded_command};
pub(crate) use style::Style;
pub(crate) use time::{BudgetBound, Deadline, at_least_timestamp, now_unix};

/// Maximum concurrent blocking operations across server request permits and
/// sync worker-pool threads, so one sync never starves concurrent tool calls.
pub(crate) const MAX_CONCURRENT_BLOCKING_OPS: usize = 8;

/// Returns whether an externally supplied UTF-8 string exceeds its byte cap.
pub(crate) fn exceeds_input_bytes(value: &str, max_bytes: usize) -> bool {
    value.len() > max_bytes
}

/// Returns a compact BLAKE3 hex identifier for correlating values across stderr
/// log lines without printing long raw strings or full filesystem paths.
pub(crate) fn redacted_log_id(value: impl AsRef<[u8]>) -> String {
    blake3::hash(value.as_ref()).to_hex().to_string()
}

/// Returns a stable BLAKE3 hex identifier for correlating a filesystem path
/// across structured server log lines.
pub(crate) fn redacted_path(path: &std::path::Path) -> String {
    redacted_log_id(path.to_string_lossy().as_bytes())
}

/// Serializes tests that mutate process-global environment variables
/// (`SLUGTHUG_HOME`, `HOME`, `SLUGAUDIT_*`) so parallel test threads cannot
/// observe each other's mutations.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "the thread panicked with a non-string payload".to_owned()
    }
}

/// Joins a test thread, converting any panic payload into its text message.
#[cfg(test)]
pub(crate) fn join_thread<T>(handle: std::thread::JoinHandle<T>) -> Result<T, String> {
    handle
        .join()
        .map_err(|payload| panic_payload_message(payload.as_ref()))
}

/// Joins a test thread and reports any panic through [`slugaudit_test_check::Check`].
#[cfg(test)]
#[track_caller]
pub(crate) fn join_checked<T>(
    handle: std::thread::JoinHandle<T>,
    what: &str,
) -> slugaudit_test_check::TestResult<T> {
    use slugaudit_test_check::Check as _;
    join_thread(handle).check(what)
}

/// Joins a scoped test thread and reports any panic through [`slugaudit_test_check::Check`].
#[cfg(test)]
#[track_caller]
pub(crate) fn join_scoped_checked<T>(
    handle: std::thread::ScopedJoinHandle<'_, T>,
    what: &str,
) -> slugaudit_test_check::TestResult<T> {
    use slugaudit_test_check::Check as _;
    handle
        .join()
        .map_err(|payload| panic_payload_message(payload.as_ref()))
        .check(what)
}

static SESSION_ID: std::sync::OnceLock<uuid::Uuid> = std::sync::OnceLock::new();

/// Returns the current process session UUID, allocating a v4 UUID on first call.
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

impl From<LockPoisoned> for rmcp::ErrorData {
    fn from(error: LockPoisoned) -> Self {
        Self::internal_error(format!("{error}; restart the SlugAudit server"), None)
    }
}

/// Acquires a mutex guard, logging once and returning [`LockPoisoned`] if an
/// earlier holder panicked.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when an earlier holder panicked.
pub(crate) fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    mutex_name: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, LockPoisoned> {
    mutex.lock().map_err(|_poisoned| {
        tracing::error!(
            mutex = mutex_name,
            "mutex poisoned by a panic in an earlier holder"
        );
        LockPoisoned { mutex: mutex_name }
    })
}

/// Hex-encodes a byte slice as a lowercase hex string.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|&byte| [hex_digit(byte >> 4), hex_digit(byte & 0x0f)])
        .collect()
}

fn hex_digit(nibble: u8) -> char {
    match b"0123456789abcdef".get(usize::from(nibble & 0x0f)).copied() {
        Some(digit) => char::from(digit),
        None => '0',
    }
}

/// Increments a saturating stats tally (`u64::MAX` means "at least this many").
pub(crate) fn tally(count: &mut u64) {
    // KEEP saturating: a stats tally has a designed ceiling; at u64::MAX it reads "at least this many" and never wraps to a small number
    *count = count.saturating_add(1);
}

/// Measures the serialized JSON byte length of `value` without allocating.
pub(crate) fn serialized_len<T: serde::Serialize>(value: &T) -> Result<usize, serde_json::Error> {
    let mut counter = LengthCounter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

/// Discarding `Write` sink that counts bytes up to `usize::MAX`.
pub(crate) struct LengthCounter(pub(crate) usize);

impl std::io::Write for LengthCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.checked_add(buf.len()).ok_or_else(|| {
            std::io::Error::other("the serialized length overflowed a byte count")
        })?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
