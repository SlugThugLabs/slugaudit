//! Tests for helpers defined in `crate::util`.
//!
//! These are the helper-level `(any T)` tests for `lock_or_recover`:
//! they exercise the helper on plain `Mutex<T>` types so any future
//! caller can replace the panic-recovery pattern with the same helper
//! without re-proving the helper's invariants. The WatchState-specific
//! poison-recovery test (using `Mutex<ProjectWatchState>`) lives in
//! `watch::state::tests` because it verifies the recovery in the context
//! where the helper is actually used.

use crate::util::{at_least_timestamp, hex_encode, lock_or_recover, now_unix};
use std::sync::{Arc, Mutex};

/// A panic inside a critical section poisons the underlying Mutex; the
/// next caller must be able to recover the inner value rather than
/// panicking again. Without this guarantee, a single bug in any code
/// path that uses `lock_or_recover`-equivalent recovery would crash the
/// enclosing process on the next lock attempt. This test reproduces
/// exactly that sequence on a generic `Mutex<String>`.
#[test]
fn lock_or_recover_returns_inner_state_after_a_panic_in_a_critical_section() {
    let mutex: Mutex<String> = Mutex::new("uncorrupted".to_owned());

    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = mutex.lock().expect("unpoisoned at first");
        panic!("simulated panic inside the critical section");
    }));
    assert!(panic_result.is_err(), "the panic must have happened");

    // After the panic, a std `lock()` would fail with `PoisonError`.
    // `lock_or_recover` must succeed and yield the original value
    // (the string may be in a logically-uncertain state, which is the
    // trade-off of recovery — but it must not be lost).
    let mut guard = lock_or_recover(&mutex);
    assert_eq!(*guard, "uncorrupted");
    guard.push_str(" and recoverable");
    assert_eq!(*guard, "uncorrupted and recoverable");
}

/// `lock_or_recover` on a healthy, unpoisoned mutex must behave exactly
/// like `Mutex::lock().unwrap()` — same guard, same data, same lifecycle.
///
/// This is the regression test for the positive path: if a future change
/// to the helper breaks the lifetime/guard wiring on the non-poison
/// case, this test catches it before any production code that uses
/// `lock_or_recover` runs against poison. A generic `Mutex<Vec<u32>>`
/// is used so the test doesn't accidentally prove order-of-operations
/// specific to one caller's data structure.
#[test]
fn lock_or_recover_on_a_healthy_mutex_returns_a_normal_guard() {
    let mutex: Mutex<Vec<u32>> = Mutex::new(vec![10, 20, 30]);
    let mut guard = lock_or_recover(&mutex);
    assert_eq!(*guard, vec![10, 20, 30]);
    guard.push(40);
    assert_eq!(*guard, vec![10, 20, 30, 40]);

    drop(guard);
    // Re-acquiring must continue to work — recovery is not sticky.
    let guard2 = lock_or_recover(&mutex);
    assert_eq!(*guard2, vec![10, 20, 30, 40]);
}

/// Even after a recovery, the helper must still propagate subsequent
/// panics — recovery is not sticky and doesn't silently swallow errors
/// inside the recovered critical section. Use `lock_or_recover` inside
/// the panicking closure itself so iter-2 can acquire the lock at all
/// (std `lock().expect()` would fail-PoisonError on iter-2 because
/// iter-1 panicked while holding the guard). Three iterations prove the
/// helper distinguishes "recover" from "skip the call".
#[test]
fn lock_or_recover_recovers_multiple_sequential_poisons() {
    let inner: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));
    for iteration in 1..=3_i64 {
        let inner_clone = Arc::clone(&inner);
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut g = lock_or_recover(&inner_clone);
            *g = iteration;
            panic!("simulated panic {iteration}");
        }));
        assert!(panic_result.is_err());

        let mut guard = lock_or_recover(&inner);
        assert_eq!(
            *guard, iteration,
            "value set before the panic must survive a recovery, even after multiple poisonings"
        );
        *guard += 1000;
    }
}

#[test]
fn hex_encode_renders_lowercase_hex() {
    assert_eq!(hex_encode(b""), "");
    assert_eq!(hex_encode(b"\x00\xff\x10"), "00ff10");
    assert_eq!(hex_encode(b"hello"), "68656c6c6f");
}

#[test]
fn now_unix_is_nonzero_and_never_moves_backwards() {
    let first = now_unix();
    assert!(first > 0, "the epoch must be in the past");
    let second = now_unix();
    assert!(
        second >= first,
        "consecutive reads must never move backwards"
    );
}

#[test]
fn at_least_timestamp_never_drops_below_the_persisted_value() {
    assert_eq!(at_least_timestamp(100, Some(200)), 200);
    assert_eq!(at_least_timestamp(200, Some(100)), 200);
    assert_eq!(at_least_timestamp(100, None), 100);
}
