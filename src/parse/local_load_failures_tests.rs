use super::{GLOBAL_FAILURES, LocalLoadFailures, is_locally_failed, record_local_load_failure};
use slugaudit_test_check::{Check, TestResult, ensure};
use std::sync::Arc;
use std::thread;

#[test]
fn tracks_per_language_on_current_thread_and_across_threads() -> TestResult {
    ensure(
        !GLOBAL_FAILURES
            .contains("no_such_language_for_global_failures_test")
            .check("global failures initial state")?,
        "process-wide GLOBAL_FAILURES starts empty",
    )?;
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

    let shared = Arc::new(LocalLoadFailures::new());
    let writer = Arc::clone(&shared);
    let handle = thread::spawn(move || writer.record("zig"));
    crate::util::join_thread(handle)
        .check("writer joined")?
        .check("record zig")?;
    ensure(
        shared.contains("zig").check("zig across threads")?,
        "reader observes failure recorded on another thread",
    )?;
    ensure(
        !shared.contains("ocaml").check("ocaml across threads")?,
        "unrecorded language stays false across threads",
    )?;
    Ok(())
}
