//! A poisoned registry or connection and an unusable clock reach the caller as
//! their own errors, logged once, with the restart or fix-the-clock remedy and
//! not the result store's "disk is full" advice.

use super::super::cell::Take;
use super::super::harness_tests::{GRACE_SECS, fixture, write_rows};
use super::*;
use crate::store::test_capture::capture_at_level;
use crate::util::ClockError;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::Level;

/// Poisons `mutex` the way a bug would: a holder unwinds inside its critical
/// section. `resume_unwind` starts the unwind without the `panic!` macro.
fn poison<T>(mutex: &Mutex<T>) {
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TestResult {
        let _guard = mutex.lock().check("lock the fresh mutex")?;
        std::panic::resume_unwind(Box::new("simulated panic inside the critical section"));
    }));
    assert!(unwound.is_err(), "the holder must have unwound");
    assert!(mutex.is_poisoned(), "the unwind must poison the mutex");
}

/// A real clock error: a time before the Unix epoch.
fn clock_error() -> TestResult<ClockError> {
    let before = std::time::UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .check("a time before the epoch")?;
    crate::clock::unix_seconds(before).check_err("a pre-epoch time is a clock error")
}

/// A store whose clock fails while `failing` is set.
fn store_with_failing_clock(
    dir: &Path,
    failing: &Arc<AtomicBool>,
) -> TestResult<(ResultStore, Arc<AtomicBool>)> {
    let error = clock_error()?;
    let flag = Arc::clone(failing);
    let clock: Clock = Arc::new(move || {
        if flag.load(Ordering::SeqCst) {
            Err(error.clone())
        } else {
            Ok(1_000)
        }
    });
    let store = ResultStore::open_with_clock(
        dir,
        Uuid::new_v4(),
        ResultStoreLimits {
            max_bytes: 1 << 30,
            active_grace_secs: GRACE_SECS,
        },
        clock,
    )
    .check("open the store")?;
    Ok((store, Arc::clone(failing)))
}

fn lines_containing(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
}

/// The wire form of a lock error: the plain lock error's internal error with
/// no result store code.
fn assert_restart_error(error: &ResultStoreError, mutex: &str) {
    assert!(matches!(error, ResultStoreError::Lock(_)), "{error:?}");
    let mapped = crate::error_codes::store_error(error);
    assert_eq!(mapped.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(mapped.data, None);
    assert_eq!(
        mapped.message,
        format!(
            "mutex {mutex} was poisoned by a panic in an earlier holder; \
             restart the SlugAudit server"
        )
    );
}

/// The wire form of a clock error: the plain clock error's internal error with
/// no result store code.
fn assert_clock_error(error: &ResultStoreError) {
    assert!(matches!(error, ResultStoreError::Clock(_)), "{error:?}");
    let mapped = crate::error_codes::store_error(error);
    assert_eq!(mapped.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(mapped.data, None);
    assert!(
        mapped
            .message
            .starts_with("the system clock is before the Unix epoch"),
        "{}",
        mapped.message
    );
    assert!(
        mapped.message.ends_with("; fix the system clock"),
        "{}",
        mapped.message
    );
    assert!(!mapped.message.contains("disk is full"));
}

#[test]
fn a_poisoned_registry_fails_stats_with_the_restart_error_logged_once() -> TestResult {
    let fx = fixture(1 << 30)?;
    poison(&fx.store.registry);

    let (outcome, logs) = capture_at_level(|| fx.store.stats(), Level::INFO)?;

    let error = outcome.check_err("a poisoned registry fails the snapshot")?;
    assert_restart_error(&error, "result_registry");
    assert_eq!(lines_containing(&logs, "result_registry"), 1, "{logs}");
    assert_eq!(lines_containing(&logs, "result store unavailable"), 0);
    Ok(())
}

#[test]
fn a_poisoned_registry_is_the_error_of_every_refusal_and_no_cursor_is_logged_refused() -> TestResult
{
    let fx = fixture(1 << 30)?;
    poison(&fx.store.registry);
    let id = ResultId(Uuid::new_v4());

    let ((foreign, unknown, too_large), logs) = capture_at_level(
        || {
            (
                fx.store.refuse_foreign_tool(),
                fx.store.decode_cursor("not-a-cursor").err(),
                fx.store.refuse_too_large(id, 10),
            )
        },
        Level::INFO,
    )?;

    assert_restart_error(&foreign, "result_registry");
    let unknown = unknown.check("a malformed token is refused")?;
    assert_restart_error(&unknown, "result_registry");
    assert_restart_error(&too_large, "result_registry");
    assert_eq!(lines_containing(&logs, "cursor refused"), 0, "{logs}");
    assert_eq!(lines_containing(&logs, "larger than the whole budget"), 0);
    assert_eq!(lines_containing(&logs, "result_registry"), 3, "{logs}");
    Ok(())
}

#[test]
fn an_unknown_cursor_on_a_healthy_store_is_unknown_and_logged_refused_once() -> TestResult {
    let fx = fixture(1 << 30)?;

    let (outcome, logs) = capture_at_level(|| fx.store.decode_cursor("not-a-cursor"), Level::INFO)?;

    let error = outcome.check_err("a malformed token is refused")?;
    assert!(matches!(error, ResultStoreError::Unknown), "{error:?}");
    assert_eq!(
        lines_containing(&logs, "cursor refused: unknown"),
        1,
        "{logs}"
    );
    assert_eq!(fx.store.stats().check("stats")?.unknown_cursors, 1);
    Ok(())
}

#[test]
fn a_poisoned_connection_fails_read_page_with_the_restart_error_logged_once() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 3, 4).check("write a result")?;
    let entry = fx.store.lookup(handle.id).check("look the result up")?;
    poison(&entry.conn);

    let (outcome, logs) = capture_at_level(
        || fx.store.read_page(handle.id, 0, |_| Take::Yes),
        Level::INFO,
    )?;

    let error = outcome.check_err("a poisoned connection fails the read")?;
    assert_restart_error(&error, "result_entry_connection");
    assert_eq!(
        lines_containing(&logs, "result_entry_connection"),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn a_failing_clock_fails_stats_with_the_fix_the_clock_error_logged_once() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let failing = Arc::new(AtomicBool::new(true));
    let (store, _) = store_with_failing_clock(dir.path(), &failing)?;

    let (outcome, logs) = capture_at_level(|| store.stats(), Level::INFO)?;

    assert_clock_error(&outcome.check_err("an unusable clock fails the snapshot")?);
    assert_eq!(
        lines_containing(&logs, "result store clock unusable"),
        1,
        "{logs}"
    );
    assert_eq!(lines_containing(&logs, "result store unavailable"), 0);
    Ok(())
}

#[test]
fn a_failing_clock_fails_admission_and_read_page_with_the_fix_the_clock_error() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let failing = Arc::new(AtomicBool::new(false));
    let (store, failing) = store_with_failing_clock(dir.path(), &failing)?;
    let handle = write_rows(&store, 3, 4).check("write while the clock works")?;
    failing.store(true, Ordering::SeqCst);

    let read = store.read_page(handle.id, 0, |_| Take::Yes);
    let written = write_rows(&store, 3, 4);

    assert_clock_error(&read.check_err("a failing clock fails the read")?);
    let written = written.check_err("a failing clock fails the write")?;
    assert_clock_error(&written);
    Ok(())
}
