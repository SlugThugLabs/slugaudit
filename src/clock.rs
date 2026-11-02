//! Clock and duration conversions whose failure modes are returned, logged or
//! saturate explicitly instead of being discarded.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A system clock that cannot be stated in Unix seconds, so no timestamp is
/// written from it.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ClockError {
    /// The clock reads a time before the Unix epoch.
    #[error("the system clock is before the Unix epoch: {0}")]
    BeforeEpoch(#[source] std::time::SystemTimeError),
    /// The clock reads a time past the `i64` range of Unix seconds.
    #[error("the system clock is past the i64 range of Unix seconds: {0}")]
    OutOfRange(#[source] std::num::TryFromIntError),
}

/// Whole seconds since the Unix epoch for `time`.
///
/// # Errors
///
/// Returns [`ClockError`] for a time before the epoch or past the `i64`
/// range.
pub(crate) fn unix_seconds(time: SystemTime) -> Result<i64, ClockError> {
    let elapsed = time
        .duration_since(UNIX_EPOCH)
        .map_err(ClockError::BeforeEpoch)?;
    whole_seconds(elapsed)
}

/// Whole seconds in `elapsed`, or an error when they exceed the `i64` range.
fn whole_seconds(elapsed: Duration) -> Result<i64, ClockError> {
    i64::try_from(elapsed.as_secs()).map_err(ClockError::OutOfRange)
}

/// A clock error becomes an internal tool error: nothing the caller sends can
/// fix the host clock.
impl From<ClockError> for rmcp::ErrorData {
    fn from(error: ClockError) -> Self {
        Self::internal_error(format!("{error}; fix the system clock"), None)
    }
}

/// Unix-epoch seconds for a file's mtime, or `None` when the platform cannot
/// report one (or reports a pre-epoch or out-of-range time); the cause is
/// logged. Recorded for reporting and for the stat-derived identity of skipped
/// files; it is never used to decide whether a file changed (content hash does).
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
            tracing::warn!(error = %error, "file mtime unusable; recording it as unknown");
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
    fn a_pre_epoch_clock_is_an_error() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::CheckErr;

        let error = unix_seconds(before_epoch()).check_err("a pre-epoch clock is an error")?;

        assert!(matches!(error, ClockError::BeforeEpoch(_)), "{error:?}");
        assert!(
            error
                .to_string()
                .starts_with("the system clock is before the Unix epoch: "),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn a_clock_past_the_i64_range_is_an_error() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::CheckErr;

        let error = whole_seconds(Duration::from_secs(i64::MAX.unsigned_abs() + 1))
            .check_err("a duration past i64 is an error")?;

        assert!(matches!(error, ClockError::OutOfRange(_)), "{error:?}");
        Ok(())
    }

    #[test]
    fn a_clock_after_the_epoch_converts_to_whole_seconds() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::Check;
        let at = UNIX_EPOCH + Duration::from_millis(42_999);

        assert_eq!(unix_seconds(at).check("convert")?, 42);
        Ok(())
    }

    #[test]
    fn a_clock_error_becomes_an_internal_tool_error() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::CheckErr;
        let error: rmcp::ErrorData = unix_seconds(before_epoch())
            .check_err("a pre-epoch clock is an error")?
            .into();

        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert!(
            error.message.ends_with("; fix the system clock"),
            "{}",
            error.message
        );
        Ok(())
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
