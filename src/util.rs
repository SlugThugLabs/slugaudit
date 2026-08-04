//! Small shared helpers used across multiple modules, kept here to avoid
//! drifting duplicate implementations.
//!
//! This module is intentionally narrow: only helpers with no more specific
//! home belong here. If a new helper clearly belongs to an existing module
//! (e.g. a hashing utility belongs in `sync::hash`, a time utility belongs
//! wherever the majority of its callers live), put it there instead of
//! defaulting to `util`. This file should not become a dumping ground.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static LAST_UNIX_TIME: AtomicI64 = AtomicI64::new(0);

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
