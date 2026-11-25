use super::{
    FetchSlot, is_locally_failed, load_failed_retry_due, record_fetch_failure,
    record_local_load_failure, test_controls, with_fetch_slot,
};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

#[test]
fn with_fetch_slot_blocks_uncached_during_cooldown_but_allows_cached() -> TestResult {
    test_controls::set_grammar_cached(false);
    record_fetch_failure().check("record fetch failure")?;

    let mut invoked = false;
    let blocked = with_fetch_slot("python", || {
        invoked = true;
        Ok(1u8)
    })
    .check("with_fetch_slot when uncached and cooling down")?;
    ensure_eq(
        &blocked,
        &FetchSlot::Blocked,
        "uncached language is blocked during cooldown",
    )?;
    ensure(!invoked, "work must not run while blocked")?;

    test_controls::set_grammar_cached(true);
    let ran = with_fetch_slot("python", || Ok(42u8)).check("with_fetch_slot when cached")?;
    ensure_eq(
        &ran,
        &FetchSlot::Ran(42u8),
        "cached language bypasses cooldown and runs",
    )?;
    Ok(())
}

#[test]
fn with_fetch_slot_serializes_concurrent_uncached_fetches() -> TestResult {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let in_flight = Arc::clone(&in_flight);
        let max_seen = Arc::clone(&max_seen);
        handles.push(thread::spawn(move || -> TestResult {
            test_controls::set_grammar_cached(false);
            let slot = with_fetch_slot("python", || {
                // KEEP saturating: test spawns 4 threads so in_flight never approaches usize::MAX
                let current = in_flight.fetch_add(1, Ordering::SeqCst).saturating_add(1);
                max_seen.fetch_max(current, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(10));
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .check("concurrent with_fetch_slot")?;
            ensure_eq(&slot, &FetchSlot::Ran(()), "uncached worker ran")?;
            Ok(())
        }));
    }

    for handle in handles {
        crate::util::join_thread(handle).check("worker thread did not panic")??;
    }
    ensure_eq(
        &max_seen.load(Ordering::SeqCst),
        &1usize,
        "at most one uncached grammar fetch runs at a time",
    )?;
    Ok(())
}

#[test]
fn record_fetch_failure_never_moves_timestamp_backwards() -> TestResult {
    record_fetch_failure().check("initial failure")?;
    let first = super::state::with_last_failure(|last| *last)
        .check("read first failure")?
        .check("first failure present")?;
    test_controls::elapse_cooldown();
    record_fetch_failure().check("later failure")?;
    let second = super::state::with_last_failure(|last| *last)
        .check("read second failure")?
        .check("second failure present")?;
    ensure(
        second >= first,
        "failure timestamp must be monotonically non-decreasing",
    )?;
    Ok(())
}

#[test]
fn record_local_load_failure_tracks_per_language_without_closing_fetch_gate() -> TestResult {
    ensure(
        !is_locally_failed("rust").check("initial rust state")?,
        "rust starts not locally failed",
    )?;
    record_local_load_failure("rust").check("record rust local failure")?;
    ensure(
        is_locally_failed("rust").check("rust after local failure")?,
        "rust is marked locally failed",
    )?;
    ensure(
        !is_locally_failed("python").check("python state")?,
        "other languages remain unaffected",
    )?;
    ensure(
        load_failed_retry_due().check("fetch gate state")?,
        "local load failure does not close the fetch gate",
    )?;
    Ok(())
}
