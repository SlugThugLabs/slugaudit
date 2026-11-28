//! Tests for helpers defined in `crate::util`.
//!
//! The `lock` tests are helper-level `(any T)` tests: they exercise the
//! helper on plain `Mutex<T>` types, so no caller re-proves its invariants.

use crate::util::{
    CappedReadError, Style, at_least_timestamp, exceeds_input_bytes, hex_encode, lock, now_unix,
    read_file_capped_bytes,
};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::sync::{Arc, Mutex};

#[test]
fn capped_byte_read_returns_a_file_exactly_at_the_cap() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("at-cap.bin");
    std::fs::write(&path, vec![b'x'; 1024]).check("write fixture")?;

    let bytes = read_file_capped_bytes(&path, 1024).check("at-cap file reads")?;

    ensure_eq(&bytes.len(), &1024, "the entire 1024-byte file is returned")?;
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
            ensure_eq(&limit, &1024, "reported limit matches the configured cap")?;
            ensure_eq(&observed, &1025, "reported observed size matches the file")?;
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
            ensure_eq(
                &error.kind(),
                &std::io::ErrorKind::NotFound,
                "missing file preserves NotFound error kind",
            )?;
        }
        other @ CappedReadError::TooLarge { .. } => {
            return Err(format!("expected Io, got {other:?}")).check("unexpected outcome");
        }
    }
    Ok(())
}

/// Opening a FIFO blocks until a writer appears, so a capped read refuses
/// one from metadata. The read runs on a thread so a regression fails the
/// test instead of hanging the suite. `mkfifo(1)` makes the fixture because
/// std has no safe FIFO constructor and `unsafe` is forbidden; it is run by
/// absolute path under the env lock because other tests rewrite `PATH`.
#[cfg(unix)]
#[test]
fn capped_byte_read_refuses_a_fifo_without_blocking() -> TestResult {
    let _env = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let mkfifo = ["/usr/bin/mkfifo", "/bin/mkfifo"]
        .into_iter()
        .find(|candidate| std::path::Path::new(candidate).exists())
        .check("mkfifo is installed at /usr/bin or /bin")?;
    let directory = tempfile::tempdir().check("temp dir")?;
    let path = directory.path().join("pipe");
    let status = std::process::Command::new(mkfifo)
        .arg(&path)
        .status()
        .check("run mkfifo")?;
    ensure(
        status.success(),
        format!("mkfifo makes the fixture: {status}"),
    )?;

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(read_file_capped_bytes(&path, 1024).map(|_| ())));
    let outcome = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .check("a capped read of a FIFO returns instead of blocking")?;

    match outcome.check_err("a FIFO is not read")? {
        CappedReadError::Io(error) => ensure_eq(
            &error.kind(),
            &std::io::ErrorKind::InvalidInput,
            "a FIFO is refused as not a regular file",
        ),
        other @ CappedReadError::TooLarge { .. } => {
            Err(format!("expected Io, got {other:?}")).check("unexpected outcome")
        }
    }
}

#[test]
fn capped_byte_read_refuses_a_directory() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;

    match read_file_capped_bytes(directory.path(), 1024).check_err("a directory is not read")? {
        CappedReadError::Io(error) => ensure_eq(
            &error.kind(),
            &std::io::ErrorKind::InvalidInput,
            "a directory is refused as not a regular file",
        ),
        other @ CappedReadError::TooLarge { .. } => {
            Err(format!("expected Io, got {other:?}")).check("unexpected outcome")
        }
    }
}

/// Poisons `mutex` the way a bug would: a holder unwinds inside its critical
/// section. `resume_unwind` starts the unwind without the `panic!` macro.
fn poison<T>(mutex: &Mutex<T>) -> TestResult {
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TestResult {
        let _guard = mutex.lock().check("lock the fresh mutex")?;
        std::panic::resume_unwind(Box::new("simulated panic inside the critical section"));
    }));
    ensure(unwound.is_err(), "the holder must have unwound")?;
    ensure(
        mutex.is_poisoned(),
        "the unwind must have poisoned the mutex",
    )?;
    Ok(())
}

/// A panic inside a critical section poisons the mutex. The next caller gets
/// an error naming the mutex, never the possibly half-updated value.
#[test]
fn lock_on_a_poisoned_mutex_is_an_error_naming_the_mutex() -> TestResult {
    let mutex: Mutex<String> = Mutex::new("uncorrupted".to_owned());
    poison(&mutex)?;

    let error = lock(&mutex, "util_tests_string").check_err("a poisoned mutex is an error")?;

    ensure_eq(
        &error,
        &crate::util::LockPoisoned {
            mutex: "util_tests_string",
        },
        "lock error carries the poisoned mutex label",
    )?;
    ensure_eq(
        &error.to_string(),
        &"mutex util_tests_string was poisoned by a panic in an earlier holder",
        "formatted lock error names the poisoned mutex",
    )?;
    Ok(())
}

/// The poisoning is logged where it is detected: one error event per failed
/// `lock`, naming the mutex, and none for a healthy mutex.
#[test]
fn lock_logs_a_poisoned_mutex_once_per_detection_and_a_healthy_one_never() -> TestResult {
    let healthy: Mutex<u8> = Mutex::new(1);
    let poisoned: Mutex<u8> = Mutex::new(2);
    poison(&poisoned)?;

    let (outcome, logs) = crate::util::test_capture::capture_at_level(
        || {
            let first = lock(&healthy, "util_tests_healthy").map(|guard| *guard);
            let second = lock(&poisoned, "util_tests_logged").map(|guard| *guard);
            (first, second)
        },
        tracing::Level::INFO,
    )?;

    ensure_eq(
        &outcome.0.check("the healthy mutex locks")?,
        &1,
        "healthy mutex returns its guarded value",
    )?;
    outcome.1.check_err("the poisoned mutex fails")?;
    let events: Vec<&str> = logs.lines().collect();
    ensure_eq(&events.len(), &1, &logs)?;
    let event = events.first().check("the one logged event")?;
    ensure(event.contains("ERROR"), &logs)?;
    ensure(event.contains("util_tests_logged"), &logs)?;
    ensure(
        event.contains("mutex poisoned by a panic in an earlier holder"),
        "error log explains that an earlier holder panicked",
    )?;
    Ok(())
}

/// `lock` on a healthy, unpoisoned mutex must behave exactly like
/// `Mutex::lock()` — same guard, same data, same lifecycle.
#[test]
fn lock_on_a_healthy_mutex_returns_a_normal_guard() -> TestResult {
    let mutex: Mutex<Vec<u32>> = Mutex::new(vec![10, 20, 30]);
    let mut guard = lock(&mutex, "util_tests_vec").check("lock the healthy mutex")?;
    ensure_eq(
        &*guard,
        &vec![10, 20, 30],
        "guard exposes the initial vector",
    )?;
    guard.push(40);
    ensure_eq(
        &*guard,
        &vec![10, 20, 30, 40],
        "guard reflects in-place mutation",
    )?;

    drop(guard);
    let guard2 = lock(&mutex, "util_tests_vec").check("lock it again")?;
    ensure_eq(
        &*guard2,
        &vec![10, 20, 30, 40],
        "subsequent lock sees the persisted mutation",
    )?;
    Ok(())
}

/// Poison is permanent: every later lock of the same mutex fails, so state a
/// panic may have half-updated is never read again in this process.
#[test]
fn lock_keeps_failing_on_every_call_after_a_poison() -> TestResult {
    let mutex: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));
    poison(&mutex)?;

    for attempt in 1..=3 {
        let error = lock(&mutex, "util_tests_i64").check_err("the mutex stays poisoned")?;
        ensure_eq(
            &error.to_string(),
            &"mutex util_tests_i64 was poisoned by a panic in an earlier holder",
            format!("attempt {attempt} reports the same poison error"),
        )?;
    }
    Ok(())
}

/// The tool error for a poisoned mutex keeps the mutex label and tells the
/// caller the only remedy: a poisoned mutex stays poisoned until restart.
#[test]
fn a_poisoned_lock_becomes_an_internal_tool_error_that_says_restart() -> TestResult {
    let mutex: Mutex<u32> = Mutex::new(7);
    poison(&mutex)?;

    let error: rmcp::ErrorData = lock(&mutex, "labelled_probe_mutex")
        .check_err("a poisoned mutex is an error")?
        .into();

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "poisoned mutex maps to an internal MCP error",
    )?;
    ensure_eq(
        &error.message,
        &"mutex labelled_probe_mutex was poisoned by a panic in an earlier holder; restart the SlugAudit server",
        "tool error message names the mutex and advises restarting the server",
    )?;
    Ok(())
}

#[test]
fn hex_encode_renders_lowercase_hex() -> TestResult {
    ensure_eq(&hex_encode(b""), &"", "empty input encodes to empty string")?;
    ensure_eq(
        &hex_encode(b"\x00\xff\x10"),
        &"00ff10",
        "boundary bytes encode to lowercase two-digit hex",
    )?;
    ensure_eq(
        &hex_encode(b"hello"),
        &"68656c6c6f",
        "ASCII text encodes to expected lowercase hex",
    )?;
    Ok(())
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
    ensure_eq(
        &hex_encode(&every_byte),
        &reference(&every_byte)?,
        "all 256 byte values match std::fmt lowercase hex formatting",
    )?;

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
        ensure_eq(
            &hex_encode(&input),
            &reference(&input)?,
            format!("pseudo-random input of length {len} matches reference hex"),
        )?;
    }
    Ok(())
}

#[test]
fn now_unix_is_nonzero_and_never_moves_backwards() -> TestResult {
    let first = now_unix().check("first read")?;
    ensure(first > 0, "the epoch must be in the past")?;
    let second = now_unix().check("second read")?;
    ensure(
        second >= first,
        "consecutive reads must never move backwards",
    )?;
    Ok(())
}

#[test]
fn at_least_timestamp_never_drops_below_the_persisted_value() -> TestResult {
    ensure_eq(
        &at_least_timestamp(100, Some(200)),
        &200,
        "clock behind persisted value clamps up to persisted value",
    )?;
    ensure_eq(
        &at_least_timestamp(200, Some(100)),
        &200,
        "clock ahead of persisted value keeps the newer clock reading",
    )?;
    ensure_eq(
        &at_least_timestamp(100, None),
        &100,
        "absent persisted value uses the clock reading directly",
    )?;
    Ok(())
}

// --- Style (ANSI / TTY) ---

/// A plain `Style` must pass text through untouched — no escape sequences
/// even when the text itself would otherwise be stylable. This is the
/// safety contract that keeps redirected/piped output clean.
#[test]
fn plain_style_passes_text_through_uncolored() -> TestResult {
    let style = Style::plain();
    ensure(!style.enabled(), "plain style reports coloring disabled")?;
    ensure_eq(&style.bold("hi"), &"hi", "plain bold adds no ANSI codes")?;
    ensure_eq(&style.dim("hi"), &"hi", "plain dim adds no ANSI codes")?;
    ensure_eq(&style.cyan("hi"), &"hi", "plain cyan adds no ANSI codes")?;
    ensure_eq(&style.green("hi"), &"hi", "plain green adds no ANSI codes")?;
    ensure_eq(
        &style.yellow("hi"),
        &"hi",
        "plain yellow adds no ANSI codes",
    )?;
    Ok(())
}

#[test]
fn enabled_style_wraps_text_in_sgr_codes() -> TestResult {
    let style = Style::colored();

    ensure(style.enabled(), "colored style reports coloring enabled")?;
    ensure_eq(
        &style.bold("hi"),
        &"\x1b[1mhi\x1b[0m",
        "colored bold wraps text in SGR 1 and reset",
    )?;
    Ok(())
}

/// The exact SGR codes used by `Style` must match the documented ANSI
/// mapping (bold=1, dim=2, cyan=36, green=32, yellow=33, reset=0), so a
/// "fix" that changes one code fails here.
#[test]
fn style_uses_the_documented_sgr_codes() -> TestResult {
    let style = Style::colored();

    let expectations = [
        (style.bold("x"), "\x1b[1mx\x1b[0m"),
        (style.dim("x"), "\x1b[2mx\x1b[0m"),
        (style.cyan("x"), "\x1b[36mx\x1b[0m"),
        (style.green("x"), "\x1b[32mx\x1b[0m"),
        (style.yellow("x"), "\x1b[33mx\x1b[0m"),
    ];
    for (output, expected) in expectations {
        ensure_eq(
            &output,
            &expected,
            "styled output matches documented SGR sequence",
        )?;
    }
    Ok(())
}

#[test]
fn input_byte_bound_check_is_utf8_safe_and_exact() -> TestResult {
    ensure(
        !exceeds_input_bytes("é", 2),
        "two-byte UTF-8 character fits within a two-byte cap",
    )?;
    ensure(
        exceeds_input_bytes("é", 1),
        "two-byte UTF-8 character exceeds a one-byte cap",
    )?;
    ensure(
        !exceeds_input_bytes("abc", 3),
        "three ASCII bytes fit within a three-byte cap",
    )?;
    ensure(
        exceeds_input_bytes("abcd", 3),
        "four ASCII bytes exceed a three-byte cap",
    )?;
    Ok(())
}

#[test]
fn redacted_log_ids_are_stable_without_containing_input() -> TestResult {
    let sample = "/workspace/repo-alpha/src/lib.rs";
    let first = crate::util::redacted_log_id(sample.as_bytes());
    let second = crate::util::redacted_path(std::path::Path::new(sample));
    ensure_eq(
        &first,
        &second,
        "byte and path correlation hashes produce the same digest",
    )?;
    ensure_ne(
        &first,
        &sample,
        "correlation digest never equals the raw path",
    )?;
    ensure_eq(&first.len(), &64, "BLAKE3 hex digest is 64 characters long")?;
    Ok(())
}

#[test]
fn a_capped_read_refuses_an_oversized_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let big = directory.path().join("big.json");
    std::fs::write(&big, vec![b'x'; 2048]).check("write oversized file")?;

    let error =
        crate::util::read_file_capped(&big, 1024).check_err("an over-cap file must fail closed")?;

    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::InvalidData,
        "oversized text file fails with InvalidData",
    )?;
    ensure(
        error.to_string().contains("over the 1024-byte cap"),
        format!("the message must name the cap that refused: {error}"),
    )?;
    Ok(())
}

#[test]
fn a_capped_read_accepts_a_file_exactly_at_the_cap() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let at_cap = directory.path().join("ok.json");
    std::fs::write(&at_cap, vec![b'y'; 1024]).check("write at-cap file")?;

    let text = crate::util::read_file_capped(&at_cap, 1024).check("a file at the cap reads")?;

    ensure_eq(&text.len(), &1024, "the cap is inclusive")?;
    Ok(())
}

#[test]
fn serialized_len_measures_exact_json_bytes() -> TestResult {
    let value = serde_json::json!({"key": "value"});
    let len = crate::util::serialized_len(&value).check("serialized_len must succeed")?;
    let exact = serde_json::to_string(&value).check("to_string")?.len();
    ensure_eq(
        &len,
        &exact,
        "streaming length counter matches materialized JSON byte length",
    )?;
    Ok(())
}

#[test]
fn join_thread_returns_the_value_of_a_finished_thread() -> TestResult {
    let handle = std::thread::spawn(|| 7);

    ensure_eq(
        &crate::util::join_thread(handle).check("thread joins")?,
        &7,
        "joined thread returns its computed value",
    )?;
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

    ensure_eq(
        &literal,
        &"literal message",
        "static string panic payload is preserved",
    )?;
    ensure_eq(
        &formatted,
        &"formatted 1",
        "owned String panic payload is preserved",
    )?;
    Ok(())
}

#[test]
fn a_length_counter_past_the_maximum_fails_the_write_with_its_cause() -> TestResult {
    use std::io::Write;
    let mut counter = crate::util::LengthCounter(usize::MAX);
    let error = counter.write(b"x").check_err("the count overflows")?;
    ensure_eq(
        &error.to_string(),
        &"the serialized length overflowed a byte count".to_owned(),
        "overflow message explains the byte count overflow",
    )?;
    ensure_eq(
        &counter.0,
        &usize::MAX,
        "a failed write leaves the count unchanged",
    )?;
    let mut empty = crate::util::LengthCounter(usize::MAX);
    ensure_eq(
        &empty.write(b"").check("an empty write adds nothing")?,
        &0,
        "an empty write at the maximum succeeds",
    )?;
    Ok(())
}

#[test]
fn search_path_matches_treats_question_mark_as_one_unicode_scalar_value() -> TestResult {
    use crate::util::search_path_matches;
    for (candidate, pattern, expected) in [
        ("café", "caf?", true),
        ("café", "caf??", false),
        ("src/café/🦀.rs", "src/caf?/?.rs", true),
        ("src/café/ab.rs", "src/caf?/?.rs", false),
        ("hello🦀", "*🦀", true),
        ("é", "?*", true),
    ] {
        ensure_eq(
            &search_path_matches(candidate, pattern),
            &expected,
            format!("search_path_matches({candidate:?}, {pattern:?})"),
        )?;
    }
    Ok(())
}
