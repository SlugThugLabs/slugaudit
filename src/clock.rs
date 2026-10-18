//! Clock and duration conversions whose failure modes are logged or saturate
//! explicitly instead of being discarded.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whole seconds since the Unix epoch for `time`. A time before the epoch
/// yields `0` and one past the `i64` range yields `i64::MAX`; both are
/// deliberate recoveries, logged once with the source error.
pub(crate) fn unix_seconds(time: SystemTime) -> i64 {
    let elapsed = time.duration_since(UNIX_EPOCH).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "system clock is before the Unix epoch; using 0");
        Duration::ZERO
    });
    i64::try_from(elapsed.as_secs()).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "system clock is past the i64 range; using the maximum");
        i64::MAX
    })
}

/// Unix-epoch seconds for a file's mtime, or `None` when the platform cannot
/// report one (or reports a pre-epoch or out-of-range time); the cause is
/// logged. Stored alongside the content hash so the sync sweep can skip
/// unchanged files with one stat instead of a full read+hash.
pub(crate) fn mtime_unix_seconds(metadata: &std::fs::Metadata) -> Option<i64> {
    mtime_seconds_from(metadata.modified())
}

fn mtime_seconds_from(modified: std::io::Result<SystemTime>) -> Option<i64> {
    let seconds = modified
        .map_err(|error| error.to_string())
        .and_then(|time| {
            time.duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())
        })
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).map_err(|error| error.to_string()));
    match seconds {
        Ok(seconds) => Some(seconds),
        Err(error) => {
            tracing::warn!(error = %error, "file mtime unusable; the file is re-hashed every sweep");
            None
        }
    }
}

/// `duration` in whole milliseconds, saturating at `u64::MAX` (about 584
/// million years) instead of overflowing.
pub(crate) fn saturating_millis(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_mul(1000)
        .saturating_add(u64::from(duration.subsec_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn before_epoch() -> SystemTime {
        UNIX_EPOCH - Duration::from_secs(5)
    }

    #[test]
    fn a_pre_epoch_clock_yields_zero_and_warns() {
        let (seconds, logs) =
            crate::store::test_capture::capture_warns(|| unix_seconds(before_epoch()));

        assert_eq!(seconds, 0);
        assert!(logs.contains("before the Unix epoch"), "{logs}");
    }

    #[test]
    fn a_pre_epoch_mtime_is_none_and_warns() {
        let (mtime, logs) =
            crate::store::test_capture::capture_warns(|| mtime_seconds_from(Ok(before_epoch())));

        assert_eq!(mtime, None);
        assert!(logs.contains("mtime unusable"), "{logs}");
    }

    #[test]
    fn an_unavailable_mtime_is_none_and_warns_with_the_cause() {
        let unsupported = std::io::Error::new(std::io::ErrorKind::Unsupported, "no mtime here");

        let (mtime, logs) =
            crate::store::test_capture::capture_warns(|| mtime_seconds_from(Err(unsupported)));

        assert_eq!(mtime, None);
        assert!(logs.contains("no mtime here"), "{logs}");
    }

    #[test]
    fn a_readable_mtime_converts_to_epoch_seconds() {
        let at = UNIX_EPOCH + Duration::from_secs(42);

        assert_eq!(mtime_seconds_from(Ok(at)), Some(42));
    }

    #[test]
    fn milliseconds_are_exact_and_saturate_at_the_maximum() {
        assert_eq!(saturating_millis(Duration::from_millis(1_234)), 1_234);
        assert_eq!(saturating_millis(Duration::MAX), u64::MAX);
    }
}
