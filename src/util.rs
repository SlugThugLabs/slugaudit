//! Small shared helpers used across multiple modules, kept here to avoid
//! drifting duplicate implementations.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
