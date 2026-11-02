//! Tests for helpers defined in `crate::util`.
//!
//! The `lock` tests are helper-level `(any T)` tests: they exercise the
//! helper on plain `Mutex<T>` types, so no caller re-proves its invariants.

use crate::util::{
    CappedReadError, Style, at_least_timestamp, exceeds_input_bytes, hex_encode, lock, now_unix,
    read_file_capped_bytes,
};
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::sync::{Arc, Mutex};

#[test]
fn capped_byte_read_returns_a_file_exactly_at_the_cap() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("at-cap.bin");
    std::fs::write(&path, vec![b'x'; 1024]).check("write fixture")?;

    let bytes = read_file_capped_bytes(&path, 1024).check("at-cap file reads")?;

    assert_eq!(bytes.len(), 1024);
    Ok(())
}

#[test]
fn capped_byte_read_reports_an_oversized_file_without_reading_it_whole() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("over-cap.bin");
    std::fs::write(&path, vec![b'x'; 1025]).check("write fixture")?;

    let error = read_file_capped_bytes(&path, 1024).check_err("over-cap file is refused")?;

    match error {
        CappedReadError::TooLarge { limit, observed } => {
            assert_eq!(limit, 1024);
            assert_eq!(observed, 1025);
        }
        other @ CappedReadError::Io(_) => {
            return Err(format!("expected TooLarge, got {other:?}")).check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn capped_byte_read_keeps_not_found() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("missing.bin");

    let error = read_file_capped_bytes(&path, 1024).check_err("missing file is an I/O error")?;

    match error {
        CappedReadError::Io(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        }
        other @ CappedReadError::TooLarge { .. } => {
            return Err(format!("expected Io, got {other:?}")).check("unexpected outcome");
        }
    }
    Ok(())
}

/// Poisons `mutex` the way a bug would: a holder unwinds inside its critical
/// section. `resume_unwind` starts the unwind without the `panic!` macro.
fn poison<T>(mutex: &Mutex<T>) {
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TestResult {
        let _guard = mutex.lock().check("lock the fresh mutex")?;
        std::panic::resume_unwind(Box::new("simulated panic inside the critical section"));
    }));
    assert!(unwound.is_err(), "the holder must have unwound");
    assert!(
        mutex.is_poisoned(),
        "the unwind must have poisoned the mutex"
    );
}

/// A panic inside a critical section poisons the mutex. The next caller gets
/// an error naming the mutex, never the possibly half-updated value.
#[test]
fn lock_on_a_poisoned_mutex_is_an_error_naming_the_mutex() -> TestResult {
    let mutex: Mutex<String> = Mutex::new("uncorrupted".to_owned());
    poison(&mutex);

    let error = lock(&mutex, "util_tests_string").check_err("a poisoned mutex is an error")?;

    assert_eq!(
        error,
        crate::util::LockPoisoned {
            mutex: "util_tests_string"
        }
    );
    assert_eq!(
        error.to_string(),
        "mutex util_tests_string was poisoned by a panic in an earlier holder"
    );
    Ok(())
}

/// The poisoning is logged where it is detected: one error event per failed
/// `lock`, naming the mutex, and none for a healthy mutex.
#[test]
fn lock_logs_a_poisoned_mutex_once_per_detection_and_a_healthy_one_never() -> TestResult {
    let healthy: Mutex<u8> = Mutex::new(1);
    let poisoned: Mutex<u8> = Mutex::new(2);
    poison(&poisoned);

    let (outcome, logs) = crate::store::test_capture::capture_at_level(
        || {
            let first = lock(&healthy, "util_tests_healthy").map(|guard| *guard);
            let second = lock(&poisoned, "util_tests_logged").map(|guard| *guard);
            (first, second)
        },
        tracing::Level::INFO,
    )?;

    assert_eq!(outcome.0.check("the healthy mutex locks")?, 1);
    outcome.1.check_err("the poisoned mutex fails")?;
    let events: Vec<&str> = logs.lines().collect();
    assert_eq!(events.len(), 1, "{logs}");
    assert!(events[0].contains("ERROR"), "{logs}");
    assert!(events[0].contains("util_tests_logged"), "{logs}");
    assert!(events[0].contains("mutex poisoned by a panic in an earlier holder"));
    Ok(())
}

/// `lock` on a healthy, unpoisoned mutex must behave exactly like
/// `Mutex::lock()` — same guard, same data, same lifecycle.
#[test]
fn lock_on_a_healthy_mutex_returns_a_normal_guard() -> TestResult {
    let mutex: Mutex<Vec<u32>> = Mutex::new(vec![10, 20, 30]);
    let mut guard = lock(&mutex, "util_tests_vec").check("lock the healthy mutex")?;
    assert_eq!(*guard, vec![10, 20, 30]);
    guard.push(40);
    assert_eq!(*guard, vec![10, 20, 30, 40]);

    drop(guard);
    let guard2 = lock(&mutex, "util_tests_vec").check("lock it again")?;
    assert_eq!(*guard2, vec![10, 20, 30, 40]);
    Ok(())
}

/// Poison is permanent: every later lock of the same mutex fails, so state a
/// panic may have half-updated is never read again in this process.
#[test]
fn lock_keeps_failing_on_every_call_after_a_poison() -> TestResult {
    let mutex: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));
    poison(&mutex);

    for attempt in 1..=3 {
        let error = lock(&mutex, "util_tests_i64").check_err("the mutex stays poisoned")?;
        assert_eq!(
            error.to_string(),
            "mutex util_tests_i64 was poisoned by a panic in an earlier holder",
            "attempt {attempt}"
        );
    }
    Ok(())
}

/// The tool error for a poisoned mutex keeps the mutex label and tells the
/// caller the only remedy: a poisoned mutex stays poisoned until restart.
#[test]
fn a_poisoned_lock_becomes_an_internal_tool_error_that_says_restart() -> TestResult {
    let mutex: Mutex<u32> = Mutex::new(7);
    poison(&mutex);

    let error: rmcp::ErrorData = lock(&mutex, "labelled_probe_mutex")
        .check_err("a poisoned mutex is an error")?
        .into();

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(
        error.message,
        "mutex labelled_probe_mutex was poisoned by a panic in an earlier holder; restart the SlugAudit server"
    );
    Ok(())
}

#[test]
fn hex_encode_renders_lowercase_hex() {
    assert_eq!(hex_encode(b""), "");
    assert_eq!(hex_encode(b"\x00\xff\x10"), "00ff10");
    assert_eq!(hex_encode(b"hello"), "68656c6c6f");
}

#[test]
fn hex_encode_matches_the_format_macro_for_every_byte_and_pseudo_random_inputs() -> TestResult {
    use std::fmt::Write as _;
    fn reference(bytes: &[u8]) -> TestResult<String> {
        let mut out = String::new();
        for byte in bytes {
            write!(out, "{byte:02x}").check("write to String")?;
        }
        Ok(out)
    }
    let every_byte: Vec<u8> = (0..=255).collect();
    assert_eq!(hex_encode(&every_byte), reference(&every_byte)?);

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
        assert_eq!(hex_encode(&input), reference(&input)?, "len {len}");
    }
    Ok(())
}

#[test]
fn now_unix_is_nonzero_and_never_moves_backwards() -> TestResult {
    let first = now_unix().check("first read")?;
    assert!(first > 0, "the epoch must be in the past");
    let second = now_unix().check("second read")?;
    assert!(
        second >= first,
        "consecutive reads must never move backwards"
    );
    Ok(())
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
fn a_capped_read_refuses_an_oversized_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let big = directory.path().join("big.json");
    std::fs::write(&big, vec![b'x'; 2048]).check("write oversized file")?;

    let error =
        crate::util::read_file_capped(&big, 1024).check_err("an over-cap file must fail closed")?;

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        error.to_string().contains("over the 1024-byte cap"),
        "the message must name the cap that refused: {error}"
    );
    Ok(())
}

#[test]
fn a_capped_read_accepts_a_file_exactly_at_the_cap() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let at_cap = directory.path().join("ok.json");
    std::fs::write(&at_cap, vec![b'y'; 1024]).check("write at-cap file")?;

    let text = crate::util::read_file_capped(&at_cap, 1024).check("a file at the cap reads")?;

    assert_eq!(text.len(), 1024, "the cap is inclusive");
    Ok(())
}

#[test]
fn serialized_len_measures_exact_json_bytes() -> TestResult {
    let value = serde_json::json!({"key": "value"});
    let len = crate::util::serialized_len(&value).check("serialized_len must succeed")?;
    let exact = serde_json::to_string(&value).check("to_string")?.len();
    assert_eq!(len, exact);
    Ok(())
}

#[test]
fn join_thread_returns_the_value_of_a_finished_thread() -> TestResult {
    let handle = std::thread::spawn(|| 7);

    assert_eq!(crate::util::join_thread(handle).check("thread joins")?, 7);
    Ok(())
}

#[test]
fn join_thread_returns_a_panic_message_for_both_payload_kinds() -> TestResult {
    // `resume_unwind` unwinds with exactly the payload a `panic!` with a
    // literal (`&str`) or a formatted message (`String`) carries.
    let literal =
        std::thread::spawn(|| -> () { std::panic::resume_unwind(Box::new("literal message")) });
    let formatted = std::thread::spawn(|| -> () {
        std::panic::resume_unwind(Box::new(format!("formatted {}", 1)))
    });

    let literal = crate::util::join_thread(literal).check_err("a panic is an error")?;
    let formatted = crate::util::join_thread(formatted).check_err("a panic is an error")?;

    assert_eq!(literal, "literal message");
    assert_eq!(formatted, "formatted 1");
    Ok(())
}
