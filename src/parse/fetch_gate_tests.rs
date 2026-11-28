use super::test_controls::GateResetGuard;
use super::{
    FetchSlot, GLOBAL_GATE, GateState, load_failed_retry_due, record_fetch_failure, test_controls,
    with_fetch_slot,
};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

#[test]
fn with_fetch_slot_blocks_uncached_during_cooldown_but_allows_cached() -> TestResult {
    let _gate_guard = GateResetGuard::new();
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
    let gate = Arc::new(GateState::new_isolated());
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let gate = Arc::clone(&gate);
        let in_flight = Arc::clone(&in_flight);
        let max_seen = Arc::clone(&max_seen);
        handles.push(thread::spawn(move || -> TestResult {
            let slot = gate
                .with_fetch_slot("python", || {
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
fn gate_state_propagates_cooldown_across_threads() -> TestResult {
    let gate = Arc::new(GateState::new_isolated());
    {
        let writer = Arc::clone(&gate);
        let handle = thread::spawn(move || -> TestResult {
            writer
                .record_fetch_failure()
                .check("record fetch failure on thread A")?;
            Ok(())
        });
        crate::util::join_thread(handle).check("thread A joined")??;
    }

    let reader = Arc::clone(&gate);
    let handle = thread::spawn(move || -> TestResult {
        ensure(
            !reader
                .load_failed_retry_due()
                .check("retry due during cooldown")?,
            "thread B observes cooldown started on thread A",
        )?;
        let slot = reader
            .with_fetch_slot("zig", || Ok(7u8))
            .check("with_fetch_slot on thread B")?;
        ensure_eq(
            &slot,
            &FetchSlot::Blocked,
            "thread B is blocked by cooldown recorded on thread A",
        )?;
        Ok(())
    });
    crate::util::join_thread(handle).check("thread B joined")??;

    gate.hooks.advance_secs(test_controls::PAST_COOLDOWN_SECS);
    ensure(
        gate.load_failed_retry_due()
            .check("retry due after cooldown")?,
        "cooldown expires once PastCooldown elapses on shared GateState",
    )?;
    Ok(())
}

#[test]
fn global_and_host_backed_gate_treat_bundled_rust_as_cached() -> TestResult {
    ensure(
        GLOBAL_GATE.grammar_cached("rust"),
        "process-wide GLOBAL_GATE sees statically bundled rust as cached",
    )?;
    ensure(
        !GLOBAL_GATE.grammar_cached("no_such_language_for_gate_test"),
        "unknown language is not cached in GLOBAL_GATE",
    )?;
    ensure(
        GLOBAL_GATE
            .load_failed_retry_due()
            .check("GLOBAL_GATE retry due")?,
        "fresh GLOBAL_GATE starts open",
    )?;

    let host_gate = GateState::new();
    host_gate
        .record_fetch_failure()
        .check("close host-backed gate")?;
    let rust_slot = host_gate
        .with_fetch_slot("rust", || Ok("parsed"))
        .check("bundled rust during cooldown")?;
    ensure_eq(
        &rust_slot,
        &FetchSlot::Ran("parsed"),
        "statically bundled rust runs even while fetch gate is cooling down",
    )?;
    let uncached_slot = host_gate
        .with_fetch_slot("no_such_language_for_gate_test", || Ok("unexpected"))
        .check("uncached language during cooldown")?;
    ensure_eq(
        &uncached_slot,
        &FetchSlot::Blocked,
        "uncached language is blocked by host-backed gate cooldown",
    )?;
    Ok(())
}

#[test]
fn uncached_grammars_auto_fetch_when_gate_is_open() -> TestResult {
    let gate = GateState::new_isolated();
    let mut ran = false;
    let slot = gate
        .with_fetch_slot("python", || {
            ran = true;
            Ok(3u8)
        })
        .check("uncached language runs automatically")?;
    ensure_eq(
        &slot,
        &FetchSlot::Ran(3u8),
        "uncached language auto-fetches",
    )?;
    ensure(ran, "open gate runs download closure automatically")?;
    Ok(())
}

#[test]
fn record_fetch_failure_never_moves_timestamp_backwards() -> TestResult {
    let _gate_guard = GateResetGuard::new();
    record_fetch_failure().check("initial failure")?;
    let first = test_controls::current_gate()
        .with_last_failure(|last| *last)
        .check("read first failure")?
        .check("first failure present")?;
    test_controls::elapse_cooldown();
    record_fetch_failure().check("later failure")?;
    let second = test_controls::current_gate()
        .with_last_failure(|last| *last)
        .check("read second failure")?
        .check("second failure present")?;
    ensure(
        second >= first,
        "failure timestamp must be monotonically non-decreasing",
    )?;
    Ok(())
}

#[test]
fn local_load_failure_does_not_close_fetch_gate() -> TestResult {
    let _gate_guard = GateResetGuard::new();
    let _failures_guard = crate::parse::local_load_failures::LocalLoadFailuresGuard::clear();
    crate::parse::local_load_failures::record_local_load_failure("rust")
        .check("record rust local failure")?;
    ensure(
        load_failed_retry_due().check("fetch gate state")?,
        "local load failure does not close the fetch gate",
    )?;
    Ok(())
}
