//! Tests for helpers defined in `crate::util`.
//!
//! These are the helper-level `(any T)` tests for `lock_or_recover`:
//! they exercise the helper on plain `Mutex<T>` types so any future
//! caller can replace the panic-recovery pattern with the same helper
//! without re-proving the helper's invariants. The WatchState-specific
//! poison-recovery test (using `Mutex<ProjectWatchState>`) lives in
//! `watch::state::tests` because it verifies the recovery in the context
//! where the helper is actually used.

use crate::util::{
    CappedReadError, Style, at_least_timestamp, exceeds_input_bytes, hex_encode, lock_or_recover,
    now_unix, read_file_capped_bytes,
};
use std::sync::{Arc, Mutex};

/// A panic inside a critical section poisons the underlying Mutex; the
/// next caller must be able to recover the inner value rather than
/// panicking again. Without this guarantee, a single bug in any code
/// path that uses `lock_or_recover`-equivalent recovery would crash the
/// enclosing process on the next lock attempt. This test reproduces
/// exactly that sequence on a generic `Mutex<String>`.
#[test]
fn capped_byte_read_returns_a_file_exactly_at_the_cap() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("at-cap.bin");
    std::fs::write(&path, vec![b'x'; 1024]).expect("write fixture");

    let bytes = read_file_capped_bytes(&path, 1024).expect("at-cap file reads");

    assert_eq!(bytes.len(), 1024);
}

#[test]
fn capped_byte_read_reports_an_oversized_file_without_reading_it_whole() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("over-cap.bin");
    std::fs::write(&path, vec![b'x'; 1025]).expect("write fixture");

    let error = read_file_capped_bytes(&path, 1024).expect_err("over-cap file is refused");

    match error {
        CappedReadError::TooLarge { limit, observed } => {
            assert_eq!(limit, 1024);
            assert_eq!(observed, 1025);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn capped_byte_read_keeps_not_found() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("missing.bin");

    let error = read_file_capped_bytes(&path, 1024).expect_err("missing file is an I/O error");

    match error {
        CappedReadError::Io(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

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
    let mut guard = lock_or_recover(&mutex, "util_tests_string");
    assert_eq!(*guard, "uncorrupted");
    guard.push_str(" and recoverable");
    assert_eq!(*guard, "uncorrupted and recoverable");
}

/// `lock_or_recover` on a healthy, unpoisoned mutex must behave exactly
/// like a panicking `Mutex::lock()` — same guard, same data, same lifecycle.
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
    let mut guard = lock_or_recover(&mutex, "util_tests_vec");
    assert_eq!(*guard, vec![10, 20, 30]);
    guard.push(40);
    assert_eq!(*guard, vec![10, 20, 30, 40]);

    drop(guard);
    // Re-acquiring must continue to work — recovery is not sticky.
    let guard2 = lock_or_recover(&mutex, "util_tests_vec");
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
            let mut g = lock_or_recover(&inner_clone, "util_tests_i64");
            *g = iteration;
            panic!("simulated panic {iteration}");
        }));
        assert!(panic_result.is_err());

        let mut guard = lock_or_recover(&inner, "util_tests_i64");
        assert_eq!(
            *guard, iteration,
            "value set before the panic must survive a recovery, even after multiple poisonings"
        );
        *guard += 1000;
    }
}

/// The poison-recovery line must identify which state was recovered. An
/// operator reading "a mutex was poisoned" without a label cannot tell
/// whether the session id, the watch state, or the sampling throttle is
/// suspect.
#[test]
fn lock_or_recover_logs_the_mutex_label_it_was_given() {
    let mutex: Mutex<u32> = Mutex::new(7);
    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = mutex.lock().expect("unpoisoned at first");
        panic!("simulated panic inside the labelled critical section");
    }));
    assert!(panic_result.is_err(), "the panic must have happened");

    let (value, logs) = crate::store::test_capture::capture_at_level(
        || {
            tracing::error!("capture liveness control for lock_or_recover");
            *lock_or_recover(&mutex, "labelled_probe_mutex")
        },
        tracing::Level::ERROR,
    );

    assert_eq!(value, 7, "the recovered value must be returned");
    assert!(
        logs.contains("capture liveness control for lock_or_recover"),
        "the capture must see this thread's events, captured: {logs}"
    );
    assert!(
        logs.contains("operation=\"lock_or_recover\""),
        "the recovery line must name the operation, captured: {logs}"
    );
    assert!(
        logs.contains("mutex=\"labelled_probe_mutex\""),
        "the recovery line must carry the caller's label, captured: {logs}"
    );
}

#[test]
fn hex_encode_renders_lowercase_hex() {
    assert_eq!(hex_encode(b""), "");
    assert_eq!(hex_encode(b"\x00\xff\x10"), "00ff10");
    assert_eq!(hex_encode(b"hello"), "68656c6c6f");
}

#[test]
fn hex_encode_matches_the_format_macro_for_every_byte_and_pseudo_random_inputs() {
    use std::fmt::Write as _;
    fn reference(bytes: &[u8]) -> String {
        let mut out = String::new();
        for byte in bytes {
            write!(out, "{byte:02x}").expect("write to String");
        }
        out
    }
    let every_byte: Vec<u8> = (0..=255).collect();
    assert_eq!(hex_encode(&every_byte), reference(&every_byte));

    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for len in [0_usize, 1, 2, 3, 31, 32, 33, 1000] {
        let input: Vec<u8> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        assert_eq!(hex_encode(&input), reference(&input), "len {len}");
    }
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

// --- Style (ANSI / TTY) ---

/// A plain `Style` must pass text through untouched — no escape sequences
/// even when the text itself would otherwise be stylable. This is the
/// safety contract that keeps redirected/piped output clean.
#[test]
fn plain_style_passes_text_through_uncolored() {
    let style = Style::plain();
    assert!(!style.enabled());
    assert_eq!(style.bold("hi"), "hi");
    assert_eq!(style.dim("hi"), "hi");
    assert_eq!(style.cyan("hi"), "hi");
    assert_eq!(style.green("hi"), "hi");
    assert_eq!(style.yellow("hi"), "hi");
}

#[test]
fn enabled_style_wraps_text_in_sgr_codes() {
    let style = Style::colored();

    assert!(style.enabled());
    assert_eq!(style.bold("hi"), "\x1b[1mhi\x1b[0m");
}

/// The exact SGR codes used by `Style` must match the documented ANSI
/// mapping (bold=1, dim=2, cyan=36, green=32, yellow=33, reset=0), so a
/// "fix" that changes one code fails here.
#[test]
fn style_uses_the_documented_sgr_codes() {
    let style = Style::colored();

    let expectations = [
        (style.bold("x"), "\x1b[1mx\x1b[0m"),
        (style.dim("x"), "\x1b[2mx\x1b[0m"),
        (style.cyan("x"), "\x1b[36mx\x1b[0m"),
        (style.green("x"), "\x1b[32mx\x1b[0m"),
        (style.yellow("x"), "\x1b[33mx\x1b[0m"),
    ];
    for (output, expected) in expectations {
        assert_eq!(output, expected);
    }
}

#[test]
fn input_byte_bound_check_is_utf8_safe_and_exact() {
    assert!(!exceeds_input_bytes("é", 2));
    assert!(exceeds_input_bytes("é", 1));
    assert!(!exceeds_input_bytes("abc", 3));
    assert!(exceeds_input_bytes("abcd", 3));
}

#[test]
fn redacted_log_ids_are_stable_without_containing_input() {
    let secret = "/home/alice/private-project";
    let first = crate::util::redacted_log_id(secret.as_bytes());
    let second = crate::util::redacted_path(std::path::Path::new(secret));
    assert_eq!(first, second);
    assert_ne!(first, secret);
    assert_eq!(first.len(), 64);
}

#[test]
fn canonicalize_failure_falls_back_with_a_warning() {
    let missing = std::path::Path::new("/nonexistent-slugaudit-canonical-probe");

    let (result, logs) =
        crate::store::test_capture::capture_warns(|| crate::util::canonical_or_self(missing));

    assert_eq!(result, missing, "the fallback is the path as given");
    assert!(
        logs.contains("could not canonicalize"),
        "a silent watch-key fallback is what this helper exists to prevent:\n{logs}"
    );
}

#[test]
fn canonicalize_of_an_existing_path_does_not_warn() {
    let directory = tempfile::tempdir().expect("temp dir");
    let canonical = directory
        .path()
        .canonicalize()
        .expect("canonicalize tempdir");

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-probe-canonical");
        crate::util::canonical_or_self(directory.path())
    });

    assert_eq!(result, canonical);
    assert!(
        logs.contains("control-line-probe-canonical"),
        "capture liveness control"
    );
    assert!(
        !logs.contains("could not canonicalize"),
        "existing paths must canonicalize silently:\n{logs}"
    );
}

#[test]
fn a_capped_read_refuses_an_oversized_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let big = directory.path().join("big.json");
    std::fs::write(&big, vec![b'x'; 2048]).expect("write oversized file");

    let error =
        crate::util::read_file_capped(&big, 1024).expect_err("an over-cap file must fail closed");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        error.to_string().contains("over the 1024-byte cap"),
        "the message must name the cap that refused: {error}"
    );
}

#[test]
fn a_capped_read_accepts_a_file_exactly_at_the_cap() {
    let directory = tempfile::tempdir().expect("temp dir");
    let at_cap = directory.path().join("ok.json");
    std::fs::write(&at_cap, vec![b'y'; 1024]).expect("write at-cap file");

    let text = crate::util::read_file_capped(&at_cap, 1024).expect("a file at the cap reads");

    assert_eq!(text.len(), 1024, "the cap is inclusive");
}

#[test]
fn serialized_len_measures_exact_json_bytes() {
    let value = serde_json::json!({"key": "value"});
    let len = crate::util::serialized_len(&value).expect("serialized_len must succeed");
    let exact = serde_json::to_string(&value).expect("to_string").len();
    assert_eq!(len, exact);
}
