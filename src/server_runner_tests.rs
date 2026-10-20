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

    let error = result.expect_err("a panicking tool must fail the call, not take down the process");
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("tool task failed: "),
        "the call must carry the runner's wrapper, not a bare join error: {}",
        error.message
    );
    assert!(
        error.message.contains("panicked with message \"boom\""),
        "the join error's cause must be preserved: {}",
        error.message
    );
}

/// `Semaphore::acquire_owned` fails only once the semaphore is closed. That
/// arm has its own typed error and must not run the work without a permit.
#[tokio::test]
async fn a_closed_semaphore_fails_the_call_with_its_own_typed_error_before_running_the_work() {
    let semaphore = Arc::new(Semaphore::new(4));
    semaphore.close();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let work_ran = Arc::clone(&ran);

    let result: Result<(), ErrorData> = run_blocking(
        Arc::clone(&semaphore),
        "test_closed_semaphore",
        move || {
            work_ran.store(true, Ordering::SeqCst);
            Ok(())
        },
        None,
    )
    .await;

    let error = result.expect_err("a closed semaphore must fail the call");
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(error.message, "semaphore closed");
    assert!(
        !ran.load(Ordering::SeqCst),
        "the work must not run without a permit"
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
//
// Each capturing test below names its tool with a literal only that test
// uses and counts only the lines carrying it, so "exactly one start and
// one completion per call" is a property of this test's own calls and can
// never be satisfied by a line another test's call produced.

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

/// The captured lines belonging to one tool literal. Each capturing test
/// names its tool with a literal unique to itself, so a line another test's
/// call produced can never be counted here: the assertions below then count
/// exactly the lines the test's own calls emitted.
fn lines_for_tool<'a>(logs: &'a str, tool: &str) -> Vec<&'a str> {
    let needle = format!("tool=\"{tool}\"");
    logs.lines().filter(|line| line.contains(&needle)).collect()
}

/// The completion lines of a capture for one tool literal: the single marker
/// both outcomes use, so "exactly one per call" is a countable property.
fn completion_lines<'a>(logs: &'a str, tool: &str) -> Vec<&'a str> {
    lines_for_tool(logs, tool)
        .into_iter()
        .filter(|line| line.contains("tool call completed"))
        .collect()
}

/// Every test below probes a capture with `contains`, so one escape sequence
/// anywhere in the text silently defeats all of them. The formatter colours
/// by default and interleaves escapes between a field's name, `=` and value;
/// that is what made three of these tests pass only when the ambient
/// `NO_COLOR` happened to be set — green locally, red in CI.
///
/// The control assertion comes first on purpose: an absence-only check on a
/// capture that went blind would pass while proving nothing.
#[test]
fn captured_text_carries_no_ansi_escapes() {
    let (_, logs) = capture_at_level(
        || tracing::info!(tool = "probe_ansi_escapes", "tool call completed"),
        Level::INFO,
    );
    assert!(
        logs.contains("tool=\"probe_ansi_escapes\""),
        "the field must be probeable as a plain substring, or every capture \
         assertion in this file is blind:\n{logs:?}"
    );
    assert!(
        !logs.contains('\u{1b}'),
        "an escape sequence anywhere defeats substring probes:\n{logs:?}"
    );
}

/// Two consecutive calls to the same tool must be distinguishable, and each
/// must carry the session it belongs to — otherwise a session's call
/// sequence cannot be recovered from the log at all.
#[test]
fn each_tool_call_logs_one_completion_line_with_a_session_and_its_own_call_id() {
    // Scoped to this test's own tool name: every capturing test names its
    // tool with a literal nothing else uses, so these counts can only ever
    // come from the calls made below.
    const TOOL: &str = "probe_call_sequence";
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
                        TOOL,
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

    let completions = completion_lines(&logs, TOOL);
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
        assert!(
            line.contains(&format!("tool=\"{TOOL}\"")),
            "tool name missing: {line}"
        );
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
    let started: Vec<&str> = lines_for_tool(&logs, TOOL)
        .into_iter()
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
    let _session_guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK);
    // Scoped to this test's own tool name, so the single completion line it
    // counts can only be the one its own call emitted.
    const TOOL: &str = "probe_failed_call";
    let (result, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(async {
                run_blocking(
                    Arc::new(Semaphore::new(4)),
                    TOOL,
                    || Err::<(), _>(ErrorData::invalid_params("bad request", None)),
                    None,
                )
                .await
            })
        },
        Level::INFO,
    );
    let error = result.expect_err("the tool's error must propagate");
    assert_eq!(
        error.message, "bad request",
        "the tool's own error, not a wrapper"
    );

    let completions = completion_lines(&logs, TOOL);
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
    let _session_guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK);
    const SENTINEL: &str = "SENTINEL_TOOLCALL_9147";

    // Scoped to this test's own tool name, so the completion line it counts
    // can only be the one its own call emitted.
    const TOOL: &str = "query_tool_call_logging";
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql: format!("SELECT '{SENTINEL}' AS marker FROM files"),
        offset: 0,
        expected_revision: None,
        page_bytes: None,
    };

    let (result, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(run_blocking(
                Arc::new(Semaphore::new(4)),
                TOOL,
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
        completion_lines(&logs, TOOL).len(),
        1,
        "the completion line must be captured, or this test proves nothing:\n{logs}"
    );
    assert!(
        !logs.contains(SENTINEL),
        "the query text leaked into the tool-call log:\n{logs}"
    );
}

/// A tool error's message routinely embeds user input — here the identifier
/// in SQLite's `no such column: <identifier>`. The caller must receive it,
/// but the completion line must carry only the static failure shape, or a
/// path or SQL fragment reaches the log through `error.message`.
#[test]
fn a_failed_tool_call_logs_the_static_error_class_but_never_the_message() {
    let _session_guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK);
    // Scoped to this test's own tool name so the one completion line it
    // counts can only be the one its own call emitted.
    const TOOL: &str = "query_error_logging_redaction";
    const SECRET: &str = "SENTINEL_ERROR_LOG_7231";
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql: format!("SELECT {SECRET} FROM files"),
        offset: 0,
        expected_revision: None,
        page_bytes: None,
    };

    let (result, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(run_blocking(
                Arc::new(Semaphore::new(4)),
                TOOL,
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

    let error = result
        .err()
        .expect("the unknown column must fail the query");
    assert!(
        error.message.contains(SECRET),
        "control: the caller still receives the full message: {}",
        error.message
    );

    let completions = completion_lines(&logs, TOOL);
    assert_eq!(
        completions.len(),
        1,
        "the completion line must be captured, or this test proves nothing:\n{logs}"
    );
    let line = completions[0];
    assert!(
        line.contains(&format!("tool=\"{TOOL}\"")),
        "the operation must be identifiable: {line}"
    );
    assert_eq!(logged_field(line, "outcome"), "error");
    assert_eq!(
        logged_field(line, "error_class"),
        "-32602",
        "the static JSON-RPC class must be present: {line}"
    );
    assert_eq!(
        logged_field(line, "code"),
        "none",
        "this failure carries no machine code: {line}"
    );
    assert!(
        !logs.contains(SECRET),
        "the error message leaked into the tool-call log:\n{logs}"
    );
}

/// The machine-readable code from `ErrorData.data` is the one free-form
/// failure fact that is safe to log; it must reach the completion line when
/// the producer attached one, while the message beside it stays out.
#[test]
fn a_failed_tool_call_logs_the_machine_readable_code_but_not_the_pinned_revision_id() {
    let _session_guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK);
    const TOOL: &str = "query_error_logging_machine_code";
    const STALE_REVISION: &str = "SENTINEL_STALE_REVISION_4187";
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = QueryRequest {
        path: project.path().to_string_lossy().into_owned(),
        sql: "SELECT path FROM files".to_owned(),
        offset: 0,
        expected_revision: Some(STALE_REVISION.to_owned()),
        page_bytes: None,
    };

    let (result, logs) = capture_at_level(
        || {
            current_thread_runtime().block_on(run_blocking(
                Arc::new(Semaphore::new(4)),
                TOOL,
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

    let error = result
        .err()
        .expect("a stale expected_revision must fail the query");
    assert!(
        error.message.contains(STALE_REVISION),
        "control: the caller still receives the offending revision id: {}",
        error.message
    );

    let completions = completion_lines(&logs, TOOL);
    assert_eq!(
        completions.len(),
        1,
        "the completion line must be captured, or this test proves nothing:\n{logs}"
    );
    let line = completions[0];
    assert_eq!(logged_field(line, "outcome"), "error");
    assert_eq!(
        logged_field(line, "code"),
        "stale_revision",
        "the machine-readable code must be logged: {line}"
    );
    assert_eq!(logged_field(line, "error_class"), "-32602");
    assert!(
        !logs.contains(STALE_REVISION),
        "the revision id leaked into the tool-call log:\n{logs}"
    );
}
