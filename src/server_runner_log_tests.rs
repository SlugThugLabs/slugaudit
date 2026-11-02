//! The tool-call log lines of `run_blocking`: one start and one completion
//! line per call, carrying the session and call ids, the outcome and the
//! error class, and never the call's arguments or result.

use super::*;
use crate::store::test_capture::capture_at_level;
use crate::tools::QueryRequest;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use tracing::Level;

// The run_blocking instrument is a log line, so it is pinned like one. These
// tests capture through `store::test_capture`, the same harness
// `tools::redaction_tests` uses: `capture_at_level` installs a
// thread-local subscriber, so only events emitted on the test thread are
// captured. That covers `run_blocking`'s own start and completion lines
// (the call id/session live on the span both are emitted under), not the
// internals a tool logs from inside `spawn_blocking` — which is exactly
// the boundary these tests pin.
//
// Each capturing test below names its tool with a literal only that test
// uses and counts only the lines carrying it, so "exactly one start and
// one completion per call" is a property of this test's own calls and can
// never be satisfied by a line another test's call produced.

/// The runtime the capture closures block on. Using the current-thread
/// flavor keeps every `run_blocking` line the tests assert on on the test
/// thread, under the capture subscriber.
fn current_thread_runtime() -> TestResult<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .check("test runtime")
}

/// Reads one `key=value` field out of a captured log line, accepting the
/// span-prefix form (`call_id=7}`), the event form (`call_id=7 `), and
/// quoted values (`session_id="…"`). Fails with the offending line when
/// the field is absent, so a regression names the line it lost the field on.
fn logged_field(line: &str, key: &str) -> TestResult<String> {
    let needle = format!("{key}=");
    let found = line
        .find(&needle)
        .check(&format!("field `{key}` is in log line: {line}"))?;
    let start = found + needle.len();
    let rest = &line[start..];
    let end = match rest.find([' ', '}', ',']) {
        Some(end) => end,
        None => rest.len(),
    };
    Ok(rest[..end].trim_matches('"').to_owned())
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
/// by default and interleaves escapes between a field's name, `=` and value,
/// so an ambient `NO_COLOR` would otherwise decide whether these tests pass.
///
/// The control assertion comes first on purpose: an absence-only check on a
/// capture that went blind would pass while proving nothing.
#[test]
fn captured_text_carries_no_ansi_escapes() {
    let ((), logs) = capture_at_level(
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
fn each_tool_call_logs_one_completion_line_with_a_session_and_its_own_call_id() -> TestResult {
    // Scoped to this test's own tool name: every capturing test names its
    // tool with a literal nothing else uses, so these counts can only ever
    // come from the calls made below.
    const TOOL: &str = "probe_call_sequence";
    let runtime = current_thread_runtime()?;
    let (calls, logs) = capture_at_level(
        || -> TestResult {
            runtime.block_on(async {
                let semaphore = Arc::new(Semaphore::new(4));
                for value in [1, 2] {
                    run_blocking(
                        Arc::clone(&semaphore),
                        TOOL,
                        move || Ok::<_, ErrorData>(value),
                        None,
                    )
                    .await
                    .check("call succeeds")?;
                }
                Ok(())
            })
        },
        Level::INFO,
    );
    calls?;

    let completions = completion_lines(&logs, TOOL);
    assert_eq!(
        completions.len(),
        2,
        "expected exactly one completion line per call:\n{logs}"
    );

    let session = logged_field(completions[0], "session_id")?;
    uuid::Uuid::parse_str(&session).check("the completion line carries a real session id")?;
    assert_eq!(
        session,
        logged_field(completions[1], "session_id")?,
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
        assert!(!logged_field(line, "elapsed_ms")?.is_empty(), "{line}");
    }

    let first = logged_field(completions[0], "call_id")?;
    let second = logged_field(completions[1], "call_id")?;
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
    assert_eq!(logged_field(started[0], "call_id")?, first);
    assert_eq!(logged_field(started[1], "call_id")?, second);
    Ok(())
}

/// A failed call is still one completion line, and it says so in a field
/// rather than only in the level — a sequence log grepping one marker must
/// not silently skip the calls that errored.
#[test]
fn a_failed_tool_call_logs_one_completion_line_marked_error() -> TestResult {
    // Scoped to this test's own tool name, so the single completion line it
    // counts can only be the one its own call emitted.
    const TOOL: &str = "probe_failed_call";
    let runtime = current_thread_runtime()?;
    let (result, logs) = capture_at_level(
        || {
            runtime.block_on(async {
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
    let error = result.check_err("the tool's error must propagate")?;
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
    Ok(())
}

/// The hard redaction invariant (`tools/redaction_tests.rs`) holds for the
/// lines of `run_blocking` too: a call whose argument is a sentinel must not put that
/// sentinel in any log line, the completion line included. The completion
/// line is asserted to be present as well, so the absence check cannot pass
/// vacuously.
#[test]
fn tool_call_logging_never_emits_the_calls_argument_or_result() -> TestResult {
    const SENTINEL: &str = "SENTINEL_TOOLCALL_9147";

    // Scoped to this test's own tool name, so the completion line it counts
    // can only be the one its own call emitted.
    const TOOL: &str = "query_tool_call_logging";
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let request = QueryRequest {
        path: Some(project.path().to_string_lossy().into_owned()),
        sql: format!("SELECT '{SENTINEL}' AS marker FROM files"),
        cursor: None,
        legacy_paging: crate::tools::LegacyPaging::default(),
        page_bytes: None,
    };

    let runtime = current_thread_runtime()?;
    let (result, logs) = capture_at_level(
        || {
            runtime.block_on(run_blocking(
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
    result.check("query succeeds")?;

    assert_eq!(
        completion_lines(&logs, TOOL).len(),
        1,
        "the completion line must be captured, or this test proves nothing:\n{logs}"
    );
    assert!(
        !logs.contains(SENTINEL),
        "the query text leaked into the tool-call log:\n{logs}"
    );
    Ok(())
}

/// A tool error's message routinely embeds user input — here the identifier
/// in SQLite's `no such column: <identifier>`. The caller must receive it,
/// and the completion line also logs it, exactly once, as `error`: error
/// messages are logged by design (the redaction contract covers tool
/// arguments, results and file content only).
#[test]
fn honest_log_a_failed_tool_call_logs_the_error_message_once() -> TestResult {
    // Scoped to this test's own tool name so the one completion line it
    // counts can only be the one its own call emitted.
    const TOOL: &str = "query_error_logging_redaction";
    const SECRET: &str = "SENTINEL_ERROR_LOG_7231";
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let request = QueryRequest {
        path: Some(project.path().to_string_lossy().into_owned()),
        sql: format!("SELECT {SECRET} FROM files"),
        cursor: None,
        legacy_paging: crate::tools::LegacyPaging::default(),
        page_bytes: None,
    };

    let runtime = current_thread_runtime()?;
    let (result, logs) = capture_at_level(
        || {
            runtime.block_on(run_blocking(
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
        .check("the unknown column must fail the query")?;
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
    assert_eq!(logged_field(line, "outcome")?, "error");
    assert_eq!(
        logged_field(line, "error_class")?,
        "-32602",
        "the static JSON-RPC class must be present: {line}"
    );
    assert_eq!(
        logged_field(line, "code")?,
        "sql_error",
        "the SQL error class `sql_error` is logged: {line}"
    );
    assert!(
        line.contains(SECRET),
        "the completion line must carry the error message as the cause: {line}"
    );
    assert_eq!(
        logs.lines().filter(|line| line.contains(SECRET)).count(),
        1,
        "the message must be logged on one line, not duplicated:\n{logs}"
    );
    Ok(())
}

/// The machine-readable code from `ErrorData.data` is the one free-form
/// failure fact that is safe to log; it must reach the completion line when
/// the producer attached one, while the message beside it stays out.
#[test]
fn a_failed_tool_call_logs_the_machine_readable_code_but_not_the_cursor_token() -> TestResult {
    const TOOL: &str = "query_error_logging_machine_code";
    const CURSOR_TOKEN: &str = "SENTINEL_CURSOR_TOKEN_4187";
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let request = QueryRequest {
        path: Some(project.path().to_string_lossy().into_owned()),
        sql: "SELECT path FROM files".to_owned(),
        cursor: Some(CURSOR_TOKEN.to_owned()),
        legacy_paging: crate::tools::LegacyPaging::default(),
        page_bytes: None,
    };

    let runtime = current_thread_runtime()?;
    let (result, logs) = capture_at_level(
        || {
            runtime.block_on(run_blocking(
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
        .check("an unknown cursor must fail the query")?;
    assert_eq!(
        error.data.as_ref().and_then(|data| data["code"].as_str()),
        Some("unknown_cursor"),
        "control: the caller still receives the typed error: {error:?}"
    );

    let completions = completion_lines(&logs, TOOL);
    assert_eq!(
        completions.len(),
        1,
        "the completion line must be captured, or this test proves nothing:\n{logs}"
    );
    let line = completions[0];
    assert_eq!(logged_field(line, "outcome")?, "error");
    assert_eq!(
        logged_field(line, "code")?,
        "unknown_cursor",
        "the machine-readable code must be logged: {line}"
    );
    assert_eq!(logged_field(line, "error_class")?, "-32602");
    assert!(
        !logs.contains(CURSOR_TOKEN),
        "the cursor token leaked into the tool-call log:\n{logs}"
    );
    Ok(())
}
