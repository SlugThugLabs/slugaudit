//! Clock and duration conversions whose failure modes are returned or logged
//! instead of being discarded.

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
    /// A measured duration is too long for `u64` milliseconds.
    #[error("a duration is too long for u64 milliseconds: {0}")]
    DurationMillis(#[source] std::num::TryFromIntError),
    /// Adding to a measured duration overflowed `Duration`.
    #[error("a measured duration overflowed when a millisecond was added")]
    DurationOverflow,
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
        .and_then(|time| unix_seconds(time).map_err(|error| error.to_string()));
    match seconds {
        Ok(seconds) => Some(seconds),
        Err(error) => {
            tracing::warn!(error = %error, "file mtime unusable; recording it as unknown");
            None
        }
    }
}

/// `duration` in whole milliseconds.
///
/// # Errors
///
/// Returns [`ClockError::DurationMillis`] when the milliseconds exceed
/// `u64::MAX` (about 584 million years).
pub(crate) fn duration_millis(duration: Duration) -> Result<u64, ClockError> {
    u64::try_from(duration.as_millis()).map_err(ClockError::DurationMillis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{TestResult, ensure, ensure_eq};

    fn before_epoch() -> TestResult<SystemTime> {
        use slugaudit_test_check::Check;

        UNIX_EPOCH
            .checked_sub(Duration::from_secs(5))
            .check("five seconds before the epoch is representable")
    }

    #[test]
    fn a_pre_epoch_clock_is_an_error() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::CheckErr;

        let error = unix_seconds(before_epoch()?).check_err("a pre-epoch clock is an error")?;

        ensure(
            matches!(error, ClockError::BeforeEpoch(_)),
            format!("{error:?}"),
        )?;
        ensure(
            error
                .to_string()
                .starts_with("the system clock is before the Unix epoch: "),
            format!("{error}"),
        )?;
        Ok(())
    }

    #[test]
    fn a_clock_past_the_i64_range_is_an_error() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::CheckErr;

        let error = whole_seconds(Duration::from_secs(i64::MAX.unsigned_abs() + 1))
            .check_err("a duration past i64 is an error")?;

        ensure(
            matches!(error, ClockError::OutOfRange(_)),
            format!("{error:?}"),
        )?;
        Ok(())
    }

    #[test]
    fn a_clock_after_the_epoch_converts_to_whole_seconds() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::Check;
        let at = UNIX_EPOCH + Duration::from_millis(42_999);

        ensure_eq(
            &unix_seconds(at).check("convert")?,
            &42,
            "sub-second milliseconds truncate down to 42 whole seconds",
        )?;
        Ok(())
    }

    #[test]
    fn a_clock_error_becomes_an_internal_tool_error() -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::CheckErr;
        let error: rmcp::ErrorData = unix_seconds(before_epoch()?)
            .check_err("a pre-epoch clock is an error")?
            .into();

        ensure_eq(
            &error.code,
            &rmcp::model::ErrorCode::INTERNAL_ERROR,
            "a clock failure maps to an internal MCP error code",
        )?;
        ensure(
            error.message.ends_with("; fix the system clock"),
            format!("{}", error.message),
        )?;
        Ok(())
    }

    #[test]
    fn a_pre_epoch_mtime_is_none_and_warns() -> TestResult {
        let before = before_epoch()?;
        let (mtime, logs) =
            crate::util::test_capture::capture_warns(|| mtime_seconds_from(Ok(before)))?;

        ensure_eq(&mtime, &None, "a pre-epoch mtime is recorded as unknown")?;
        ensure(logs.contains("mtime unusable"), &logs)?;
        Ok(())
    }

    #[test]
    fn an_unavailable_mtime_is_none_and_warns_with_the_cause() -> TestResult {
        let unsupported = std::io::Error::new(std::io::ErrorKind::Unsupported, "no mtime here");

        let (mtime, logs) =
            crate::util::test_capture::capture_warns(|| mtime_seconds_from(Err(unsupported)))?;

        ensure_eq(&mtime, &None, "an unavailable mtime is recorded as unknown")?;
        ensure(logs.contains("no mtime here"), &logs)?;
        Ok(())
    }

    #[test]
    fn a_readable_mtime_converts_to_epoch_seconds() -> TestResult {
        let at = UNIX_EPOCH + Duration::from_secs(42);

        ensure_eq(
            &mtime_seconds_from(Ok(at)),
            &Some(42),
            "a valid mtime converts to Unix epoch seconds",
        )?;
        Ok(())
    }

    #[test]
    fn milliseconds_are_exact_and_an_overlong_duration_is_an_error() -> TestResult {
        use slugaudit_test_check::{Check, CheckErr};
        ensure_eq(
            &duration_millis(Duration::from_millis(1_234)).check("1_234 ms fits")?,
            &1_234,
            "whole milliseconds round-trip accurately",
        )?;
        let error = duration_millis(Duration::MAX).check_err("Duration::MAX overflows u64 ms")?;
        ensure(
            matches!(error, ClockError::DurationMillis(_)),
            "an overlong duration is ClockError::DurationMillis",
        )?;
        Ok(())
    }
}
