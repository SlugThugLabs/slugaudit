use super::*;
use crate::store::test_capture::capture_at_level;
use crate::tools::QueryRequest;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::Level;

/// The sampling throttle must pass the first event, drop everything inside
/// the window, and pass again once the window elapses — the exact
/// behavior that keeps a 60 k-file import from spawning 60 k notification
/// tasks.
#[test]
fn throttle_passes_first_event_and_coalesces_within_the_window() {
    let mut throttle = Throttle::new(std::time::Duration::from_millis(100));
    let t0 = std::time::Instant::now();

    assert!(throttle.should_emit(t0), "first event always passes");
    assert!(
        !throttle.should_emit(t0 + std::time::Duration::from_millis(1)),
        "event 1 ms in is inside the window and must be dropped"
    );
    assert!(
        !throttle.should_emit(t0 + std::time::Duration::from_millis(50)),
        "event 50 ms in is still inside the window"
    );
    assert!(
        throttle.should_emit(t0 + std::time::Duration::from_millis(101)),
        "event after the window elapses passes and resets the clock"
    );
    assert!(
        !throttle.should_emit(t0 + std::time::Duration::from_millis(102)),
        "a fresh window starts at the passed event"
    );
}

#[tokio::test]
async fn run_blocking_propagates_a_successful_result() {
    let semaphore = Arc::new(Semaphore::new(4));
    let result = run_blocking(
        Arc::clone(&semaphore),
        "test",
        || Ok::<_, ErrorData>(42),
        None,
    )
    .await;
    assert_eq!(result.expect("ok"), 42);
}

#[tokio::test]
async fn run_blocking_propagates_the_tools_own_typed_error() {
    let semaphore = Arc::new(Semaphore::new(4));
    let result: Result<(), ErrorData> = run_blocking(
        Arc::clone(&semaphore),
        "test",
        || Err(ErrorData::invalid_params("bad request", None)),
        None,
    )
    .await;
    let error = result.expect_err("must propagate the tool's error, not swallow it");
    assert!(error.message.contains("bad request"));
}

#[tokio::test]
async fn a_panic_inside_the_blocking_closure_surfaces_as_a_typed_error_not_a_crash() {
    let semaphore = Arc::new(Semaphore::new(4));
    let result: Result<(), ErrorData> =
        run_blocking(Arc::clone(&semaphore), "test", || panic!("boom"), None).await;
    assert!(
        result.is_err(),
        "a panicking tool must fail the call, not take down the process"
    );
}

/// `Semaphore::acquire` blocks until a permit is free, so the number of
/// closures observed running concurrently can never exceed the semaphore's
/// permit count — not a probabilistic property but a guarantee `run_blocking`
/// must not violate (e.g. by leaking a permit or racing the counter update
/// outside the permit's held lifetime).
#[tokio::test]
async fn the_semaphore_bounds_concurrent_blocking_work() {
    const PERMITS: usize = 2;
    const TASKS: usize = 6;
    let semaphore = Arc::new(Semaphore::new(PERMITS));
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..TASKS {
        let semaphore = Arc::clone(&semaphore);
        let current = Arc::clone(&current);
        let peak = Arc::clone(&peak);
        handles.push(tokio::spawn(async move {
            run_blocking(
                Arc::clone(&semaphore),
                "test",
                move || {
                    let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    current.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, ErrorData>(())
                },
                None,
            )
            .await
        }));
    }
    for handle in handles {
        handle.await.expect("task joins").expect("work succeeds");
    }

    assert!(
        peak.load(Ordering::SeqCst) <= PERMITS,
        "observed {} concurrent blocking operations against a {}-permit semaphore",
        peak.load(Ordering::SeqCst),
        PERMITS
    );
}

// Phase 0's instrument is a log line, so it is pinned like one. These
// tests capture through `store::test_capture`, the same harness
// `tools::redaction_tests` uses: `capture_at_level` installs a
// thread-local subscriber, so only events emitted on the test thread are
// captured. That covers `run_blocking`'s own start and completion lines
// (the call id/session live on the span both are emitted under), not the
// internals a tool logs from inside `spawn_blocking` — which is exactly
// the boundary this phase adds to.

/// The runtime the capture closures block on. Using the current-thread
/// flavor keeps every `run_blocking` line the tests assert on on the test
/// thread, under the capture subscriber.
fn current_thread_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
}

/// Reads one `key=value` field out of a captured log line, accepting the
/// span-prefix form (`call_id=7}`), the event form (`call_id=7 `), and
/// quoted values (`session_id="…"`). Panics with the offending line when
/// the field is absent, so a regression names the line it lost the field on.
fn logged_field(line: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("field `{key}` missing from log line: {line}"))
        + needle.len();
    let rest = &line[start..];
    let end = rest.find([' ', '}', ',']).unwrap_or(rest.len());
    rest[..end].trim_matches('"').to_owned()
}

/// The completion lines of a capture: the single marker both outcomes use,
/// so "exactly one per call" is a countable property.
fn completion_lines(logs: &str) -> Vec<&str> {
    logs.lines()
        .filter(|line| line.contains("tool call completed"))
        .collect()
}

/// Two consecutive calls to the same tool must be distinguishable, and each
/// must carry the session it belongs to — otherwise a session's call
/// sequence cannot be recovered from the log at all.
#[test]
fn each_tool_call_logs_one_completion_line_with_a_session_and_its_own_call_id() {
    // The span stamps the live `util::SESSION_ID`, and the tests that flip
    // it (`sync::manager_meta_tests`) hold this lock while doing so — taking
    // it keeps the session stable across the two calls asserted below.
    let _session_guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK);
    let (_, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(async {
                let semaphore = Arc::new(Semaphore::new(4));
                for value in [1, 2] {
                    run_blocking(
                        Arc::clone(&semaphore),
                        "probe",
                        move || Ok::<_, ErrorData>(value),
                        None,
                    )
                    .await
                    .expect("call succeeds");
                }
            });
        },
        Level::INFO,
    );

    let completions = completion_lines(&logs);
    assert_eq!(
        completions.len(),
        2,
        "expected exactly one completion line per call:\n{logs}"
    );

    let session = logged_field(completions[0], "session_id");
    uuid::Uuid::parse_str(&session).expect("the completion line carries a real session id");
    assert_eq!(
        session,
        logged_field(completions[1], "session_id"),
        "both calls ran in one process, so they share one session id:\n{logs}"
    );

    for line in &completions {
        assert!(line.contains("tool=\"probe\""), "tool name missing: {line}");
        assert!(
            line.contains("outcome=\"success\""),
            "outcome missing: {line}"
        );
        assert!(!logged_field(line, "elapsed_ms").is_empty(), "{line}");
    }

    let first = logged_field(completions[0], "call_id");
    let second = logged_field(completions[1], "call_id");
    assert_ne!(
        first, second,
        "two consecutive calls to one tool must not share a call id:\n{logs}"
    );

    // The start line of each call carries the same correlation pair as its
    // completion line, in call order: that is what makes the sequence — not
    // just the set — readable.
    let started: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("tool call started"))
        .collect();
    assert_eq!(started.len(), 2, "one start line per call:\n{logs}");
    assert_eq!(logged_field(started[0], "call_id"), first);
    assert_eq!(logged_field(started[1], "call_id"), second);
}

/// A failed call is still one completion line, and it says so in a field
/// rather than only in the level — a sequence log grepping one marker must
/// not silently skip the calls that errored.
#[test]
fn a_failed_tool_call_logs_one_completion_line_marked_error() {
    let (result, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(async {
                run_blocking(
                    Arc::new(Semaphore::new(4)),
                    "probe",
                    || Err::<(), _>(ErrorData::invalid_params("bad request", None)),
                    None,
                )
                .await
            })
        },
        Level::INFO,
    );
    assert!(result.is_err(), "the tool's error must propagate");

    let completions = completion_lines(&logs);
    assert_eq!(
        completions.len(),
        1,
        "a failure logs exactly one completion line:\n{logs}"
    );
    assert!(
        completions[0].contains("outcome=\"error\""),
        "{}",
        completions[0]
    );
}

/// The hard redaction invariant (`tools/redaction_tests.rs`) holds for the
/// new lines too: a call whose argument is a sentinel must not put that
/// sentinel in any log line, the completion line included. The completion
/// line is asserted to be present as well, so the absence check cannot pass
/// vacuously.
#[test]
fn tool_call_logging_never_emits_the_calls_argument_or_result() {
    const SENTINEL: &str = "SENTINEL_TOOLCALL_9147";
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql: format!("SELECT '{SENTINEL}' AS marker FROM files"),
        offset: 0,
        expected_revision: None,
    };

    let (result, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(run_blocking(
                Arc::new(Semaphore::new(4)),
                "query",
                move || {
                    crate::tools::query(
                        &Parameters(request),
                        &crate::progress::NoopProgressSink,
                        &crate::sync::SourceSyncManager::default(),
                    )
                },
                None,
            ))
        },
        Level::INFO,
    );
    result.expect("query succeeds");

    assert_eq!(
        completion_lines(&logs).len(),
        1,
        "the completion line must be captured, or this test proves nothing:\n{logs}"
    );
    assert!(
        !logs.contains(SENTINEL),
        "the query text leaked into the tool-call log:\n{logs}"
    );
}
