#![forbid(unsafe_code)]

//! Proves the compiled server speaks real MCP over real stdio: the other
//! tests in this repo call Rust functions directly and never touch the
//! actual protocol. This spawns the real binary as a subprocess.

use serde_json::json;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Follows the JSON pointer `pointer` into `value`; a missing path is an error
/// naming the pointer and the value it was looked up in.
#[track_caller]
fn at<'a>(value: &'a serde_json::Value, pointer: &str) -> TestResult<&'a serde_json::Value> {
    value
        .pointer(pointer)
        .check(&format!("`{pointer}` is present in {value}"))
}

struct ServerProcess {
    /// Keeps the server's home directory alive: it points at an empty temp
    /// dir so a developer's real `~/.slugthug` config never reaches a test.
    _home: tempfile::TempDir,
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout_lines: mpsc::Receiver<String>,
    stderr_lines: mpsc::Receiver<String>,
}

impl ServerProcess {
    fn spawn() -> TestResult<Self> {
        let binary = env!("CARGO_BIN_EXE_slugaudit");
        let home = tempfile::tempdir().check("server home dir")?;
        let mut child = Command::new(binary)
            .env("SLUGTHUG_HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .check("spawn slugaudit")?;

        let stdin = child.stdin.take().check("stdin")?;
        let stdout = child.stdout.take().check("stdout")?;
        let stderr = child.stderr.take().check("stderr")?;

        let (stdout_tx, stdout_lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(err) => {
                        eprintln!("stdout reader stopped on a read error: {err}");
                        return;
                    }
                };
                if stdout_tx.send(line).is_err() {
                    return;
                }
            }
        });
        let (stderr_tx, stderr_lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(err) => {
                        eprintln!("stderr reader stopped on a read error: {err}");
                        return;
                    }
                };
                if stderr_tx.send(line).is_err() {
                    return;
                }
            }
        });

        Ok(Self {
            _home: home,
            child,
            stdin: Some(stdin),
            stdout_lines,
            stderr_lines,
        })
    }

    fn send(&mut self, json: &str) -> TestResult {
        let stdin = self.stdin.as_mut().check("stdin still open")?;
        writeln!(stdin, "{json}").check("write request line")?;
        stdin.flush().check("flush request")?;
        Ok(())
    }

    fn recv_stdout_line(&self) -> TestResult<String> {
        self.stdout_lines
            .recv_timeout(Duration::from_secs(10))
            .check("a stdout line within timeout")
    }

    /// Reads stdout lines until a JSON-RPC response with the given `id`
    /// appears. Returns the response line and any `/notifications/progress`
    /// lines that arrived before it — progress notifications have no `id`,
    /// so they cannot be the requested response, and this function drains
    /// them instead of silently dropping them between `recv_stdout_line`
    /// calls.
    fn recv_response(
        &self,
        expected_id: u64,
    ) -> TestResult<(serde_json::Value, Vec<serde_json::Value>)> {
        let mut notifications = Vec::new();
        loop {
            let line = self.recv_stdout_line()?;
            let parsed: serde_json::Value =
                serde_json::from_str(&line).check("stdout line is valid JSON")?;
            if parsed.get("id").is_some_and(|id| *id == expected_id) {
                return Ok((parsed, notifications));
            }
            // A line without an `id` matching the expected one is a
            // notification — keep it and wait for the real response.
            notifications.push(parsed);
        }
    }

    /// Sends a `tools/call` and returns the raw JSON-RPC response for `id`.
    fn tool_response(
        &mut self,
        id: u64,
        name: &str,
        arguments: &serde_json::Value,
    ) -> TestResult<serde_json::Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        self.send(&request.to_string())?;
        let (response, _) = self.recv_response(id)?;
        ensure_eq(
            at(&response, "/id")?,
            &id,
            format!("{name} response carries its request id"),
        )?;
        Ok(response)
    }

    /// Sends a `tools/call` and returns the tool result's JSON text, parsed.
    fn tool_json(
        &mut self,
        id: u64,
        name: &str,
        arguments: &serde_json::Value,
    ) -> TestResult<serde_json::Value> {
        let response = self.tool_response(id, name, arguments)?;
        let content = at(&response, "/result/content/0/text")?
            .as_str()
            .check(&format!("{name} result has text content: {response}"))?;
        serde_json::from_str(content).check(&format!("{name} result text is JSON"))
    }

    /// Drains whatever stderr lines have arrived so far (short grace
    /// period, non-blocking once quiet) into one string, for asserting
    /// specific diagnostic content landed there.
    fn stderr_snapshot(&self) -> String {
        let mut lines = Vec::new();
        while let Ok(line) = self.stderr_lines.recv_timeout(Duration::from_millis(500)) {
            lines.push(line);
        }
        lines.join("\n")
    }

    /// Initialize an enabled project directory with a single `lib.rs` fixture
    /// and return `(tempdir, slash_normalized_path)`.
    fn init_project() -> TestResult<(tempfile::TempDir, String)> {
        let project = tempfile::tempdir().check("project dir")?;
        std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
            .check("activate project")?;
        std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n")
            .check("write fixture file")?;
        let path = project.path().to_string_lossy().replace('\\', "/");
        Ok((project, path))
    }

    fn handshake(&mut self) -> TestResult {
        self.send(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client","version":"0.0.1"}}}"#,
        )?;
        let _: serde_json::Value =
            serde_json::from_str(&self.recv_stdout_line()?).check("init response")?;
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Close stdin first so the server sees EOF and exits on its own —
        // that runs its normal shutdown path (and lets coverage/profile
        // data flush). Only force-kill if it doesn't exit promptly.
        self.stdin.take();
        for _ in 0..50 {
            if self.child.try_wait().is_ok_and(|status| status.is_some()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Drop cannot return an error; a failed kill or reap is reported on
        // stderr, once each, so a leaked server process is visible.
        if let Err(error) = self.child.kill() {
            eprintln!("could not kill the test server: {error}");
        }
        if let Err(error) = self.child.wait() {
            eprintln!("could not reap the test server: {error}");
        }
    }
}

/// Asserts that `notifications` is a non-empty sequence of JSON-RPC 2.0
/// `notifications/progress` frames for `expected_token` with no `"id"` member,
/// `0.0 <= p_0 <= p_1 <= ... <= p_n == 1.0`.
fn check_monotonic_progress(
    notifications: &[serde_json::Value],
    expected_token: &str,
) -> TestResult {
    ensure(
        !notifications.is_empty(),
        format!(
            "at least one progress notification must arrive when progressToken={expected_token:?}"
        ),
    )?;

    let mut previous: Option<f64> = None;
    for (index, notification) in notifications.iter().enumerate() {
        let object = notification
            .as_object()
            .check("progress notification must be a JSON object")?;
        ensure(
            !object.contains_key("id"),
            format!(
                "progress notification {index} must omit the JSON-RPC 'id' key: {notification}"
            ),
        )?;
        ensure_eq(
            at(notification, "/method")?,
            &"notifications/progress",
            format!("stdout line {index} must be notifications/progress: {notification}"),
        )?;
        ensure_eq(
            at(notification, "/params/progressToken")?,
            &expected_token,
            format!("progressToken must round-trip on notification {index}: {notification}"),
        )?;
        let progress = at(notification, "/params/progress")?
            .as_f64()
            .check("params.progress must be a finite JSON number")?;
        ensure(
            progress.is_finite() && (0.0..=1.0).contains(&progress),
            format!(
                "progress notification {index} fraction {progress} must lie within 0.0..=1.0: {notification}"
            ),
        )?;
        if let Some(prev) = previous {
            ensure(
                prev <= progress,
                format!(
                    "progress notifications must be monotonic: notification {index} reported {progress} after {prev}"
                ),
            )?;
        }
        previous = Some(progress);
    }

    let first = notifications
        .first()
        .check("first progress notification exists")?;
    let first_progress = at(first, "/params/progress")?
        .as_f64()
        .check("first params.progress is numeric")?;
    ensure(
        first_progress >= 0.0,
        format!("first progress notification must be >= 0.0, got {first_progress}"),
    )?;

    let last = notifications
        .last()
        .check("last progress notification exists")?;
    ensure_eq(
        at(last, "/params/progress")?,
        &1.0,
        format!("final progress notification must report completion at 1.0: {last}"),
    )?;
    Ok(())
}

/// Workflow act 2: the finding bound to `lib.rs` must read stale once the
/// source changes, across a real sync on the live `server`.
fn check_finding_reads_stale_after_the_source_changes(
    server: &mut ServerProcess,
    project: &tempfile::TempDir,
    project_path: &str,
) -> TestResult {
    std::fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .check("modify source")?;

    let sync_result = server.tool_json(
        7,
        "query",
        &json!({ "path": project_path, "sql": "SELECT count(*) AS n FROM revisions" }),
    )?;
    let revision_count = at(&sync_result, "/rows/0/n")?
        .as_i64()
        .check("revision count is a number")?;
    ensure_eq(
        &revision_count,
        &2,
        "the modification must publish a second revision synchronously on the next tool call",
    )?;

    let stale_result = server.tool_json(8, "finding_read", &json!({ "path": project_path }))?;
    ensure_eq(
        at(&stale_result, "/findings/0/status")?,
        &"stale",
        "the finding must read stale once its source changed",
    )?;
    Ok(())
}

/// Structured tracing must reach stderr and nothing of the protocol may.
fn check_stderr_carries_tracing_not_protocol(server: &ServerProcess) -> TestResult {
    let stderr = server.stderr_snapshot();
    ensure(
        !stderr.is_empty(),
        "startup diagnostics should reach stderr",
    )?;
    ensure(
        stderr.contains("tool_call") && stderr.contains(r#"tool="report""#),
        format!("expected a report tool_call span in stderr, got:\n{stderr}"),
    )?;
    ensure(
        stderr.contains("report built") && stderr.contains("revision_id"),
        format!("expected report's per-tool tracing fields in stderr, got:\n{stderr}"),
    )?;
    ensure(
        !stderr.contains("\"jsonrpc\""),
        format!("stderr must never carry JSON-RPC protocol content: {stderr}"),
    )?;
    Ok(())
}

// One continuous live server process exercising a real sequence of calls
// is the point of this test — splitting it into several #[test] fns would
// each need to re-spawn and re-initialize a fresh process, losing exactly
// the "state carries correctly across a real session" property this test
// exists to prove.
#[test]
fn real_stdio_handshake_and_tool_call_stay_protocol_pure() -> TestResult {
    let (project, project_path) = ServerProcess::init_project()?;

    let mut server = ServerProcess::spawn()?;
    server.handshake()?;

    let report = server.tool_json(2, "report", &json!({ "path": project_path }))?;
    ensure_eq(
        at(&report, "/file_count")?,
        &1,
        "report file_count equals the single fixture file",
    )?;
    let languages = at(&report, "/languages")?
        .as_array()
        .check("languages array")?;
    ensure(
        languages.iter().any(|entry| {
            entry
                .get("language")
                .is_some_and(|language| language == "rust")
        }),
        "report languages array includes rust",
    )?;

    let query_result = server.tool_json(
        3,
        "query",
        &json!({ "path": project_path, "sql": "SELECT path, language FROM files" }),
    )?;
    ensure_eq(
        at(&query_result, "/rows/0/path")?,
        &"lib.rs",
        "query row 0 path is lib.rs",
    )?;
    ensure_eq(
        at(&query_result, "/rows/0/language")?,
        &"rust",
        "query row 0 language is rust",
    )?;

    let write_response = server.tool_response(
        4,
        "query",
        &json!({ "path": project_path, "sql": "DELETE FROM files" }),
    )?;
    ensure(
        at(&write_response, "/error/code")?.is_number(),
        format!(
            "a write attempt through query must come back as a typed protocol error, not succeed: {write_response}"
        ),
    )?;

    let structure_result = server.tool_json(
        5,
        "structure",
        &json!({
            "path": project_path,
            "file": "lib.rs",
            "query": "(function_item name: (identifier) @name)",
        }),
    )?;
    ensure_eq(
        at(&structure_result, "/matches/0/text")?,
        &"a",
        "structure match text is function name `a`",
    )?;
    ensure_eq(
        at(&structure_result, "/matches/0/capture_name")?,
        &"name",
        "structure match capture_name is `name`",
    )?;

    let finding_result = server.tool_json(
        6,
        "finding",
        &json!({
            "path": project_path,
            "file": "lib.rs",
            "line_start": 1,
            "line_end": 1,
            "severity": "low",
            "category": "style",
            "title": "t",
            "description": "d",
        }),
    )?;
    ensure_eq(
        at(&finding_result, "/status")?,
        &"current",
        "recorded finding status starts as current",
    )?;
    ensure(
        at(&finding_result, "/id")?
            .as_i64()
            .is_some_and(|id| id > 0),
        "recorded finding id is a positive integer",
    )?;

    check_finding_reads_stale_after_the_source_changes(&mut server, &project, &project_path)?;

    check_stderr_carries_tracing_not_protocol(&server)?;
    Ok(())
}

/// Restart behavior (release-gate item): after the server process exits, a
/// fresh server must serve the project's last revision from the persisted
/// database — the database is the source of truth across restarts, and
/// freshness re-verification still converges on the same revision for an
/// unchanged project.
#[test]
fn restart_serves_the_same_revision_from_disk() -> TestResult {
    let (_project, project_path) = ServerProcess::init_project()?;

    let report_revision = |server: &mut ServerProcess| -> TestResult<String> {
        server.handshake()?;
        let report = server.tool_json(2, "report", &json!({ "path": project_path }))?;
        Ok(at(&report, "/revision_id")?
            .as_str()
            .check("revision id")?
            .to_owned())
    };

    let mut first = ServerProcess::spawn()?;
    let first_revision = report_revision(&mut first)?;
    drop(first); // close stdin → graceful shutdown

    let mut second = ServerProcess::spawn()?;
    let second_revision = report_revision(&mut second)?;
    ensure_eq(
        &second_revision,
        &first_revision,
        "a fresh server must serve the same revision from the persisted database",
    )?;
    Ok(())
}

/// Session findings live in `findings-<id>.db` for one server process: a restarted
/// server begins with none, and `finding_clear` empties a live one and says how
/// many it removed.
#[test]
fn findings_last_one_server_process_and_finding_clear_empties_a_live_one() -> TestResult {
    let (_project, project_path) = ServerProcess::init_project()?;
    let finding = json!({
        "path": project_path,
        "file": "lib.rs",
        "line_start": 1,
        "line_end": 1,
        "severity": "low",
        "category": "style",
        "title": "t",
        "description": "d",
    });

    let mut first = ServerProcess::spawn()?;
    first.handshake()?;
    first.tool_json(2, "finding", &finding)?;
    let read = first.tool_json(3, "finding_read", &json!({ "path": project_path }))?;
    ensure_eq(
        &at(&read, "/findings")?.as_array().map(Vec::len),
        &Some(1),
        "finding_read returns the single recorded finding",
    )?;
    let cleared = first.tool_json(4, "finding_clear", &json!({ "path": project_path }))?;
    ensure_eq(
        at(&cleared, "/removed")?,
        &1,
        "finding_clear reports 1 removed finding",
    )?;
    first.tool_json(5, "finding", &finding)?;
    drop(first);

    let mut second = ServerProcess::spawn()?;
    second.handshake()?;
    let read = second.tool_json(2, "finding_read", &json!({ "path": project_path }))?;
    ensure_eq(
        &at(&read, "/findings")?.as_array().map(Vec::len),
        &Some(0),
        format!("a new server process starts with no findings: {read}"),
    )?;
    Ok(())
}

/// Two live servers on one repo each keep their own findings: the second
/// server's first call does not wipe the first server's, and `finding_clear`
/// in one leaves the other's alone.
#[test]
fn two_live_servers_on_one_repo_keep_their_own_findings() -> TestResult {
    let (_project, project_path) = ServerProcess::init_project()?;
    let finding = |title: &str| {
        json!({
            "path": project_path,
            "file": "lib.rs",
            "line_start": 1,
            "line_end": 1,
            "severity": "low",
            "category": "style",
            "title": title,
            "description": "d",
        })
    };
    let titles = |server: &mut ServerProcess, id: u64| -> TestResult<Vec<String>> {
        let read = server.tool_json(id, "finding_read", &json!({ "path": project_path }))?;
        at(&read, "/findings")?
            .as_array()
            .check("findings array")?
            .iter()
            .map(|entry| Ok(at(entry, "/title")?.as_str().check("title")?.to_owned()))
            .collect::<TestResult<Vec<String>>>()
    };

    let mut first = ServerProcess::spawn()?;
    first.handshake()?;
    first.tool_json(2, "finding", &finding("from first"))?;

    let mut second = ServerProcess::spawn()?;
    second.handshake()?;
    ensure(
        titles(&mut second, 2)?.is_empty(),
        "the second server starts with none of its own",
    )?;
    second.tool_json(3, "finding", &finding("from second"))?;

    ensure_eq(
        &titles(&mut first, 3)?,
        &["from first"],
        "first server retains its own finding",
    )?;
    ensure_eq(
        &titles(&mut second, 4)?,
        &["from second"],
        "second server retains its own finding",
    )?;
    let cleared = second.tool_json(5, "finding_clear", &json!({ "path": project_path }))?;
    ensure_eq(
        at(&cleared, "/removed")?,
        &1,
        "clearing second server removes 1 finding",
    )?;
    ensure_eq(
        &titles(&mut first, 4)?,
        &["from first"],
        "first server's finding is untouched by second server's finding_clear",
    )?;
    Ok(())
}

/// The advertised tool list has `finding_clear` and not `finding_sessions`.
#[test]
fn the_tool_list_has_finding_clear_and_no_finding_sessions() -> TestResult {
    let mut server = ServerProcess::spawn()?;
    server.handshake()?;
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)?;
    let (response, _) = server.recv_response(2)?;

    let names = at(&response, "/result/tools")?
        .as_array()
        .check("tools array")?
        .iter()
        .map(|tool| at(tool, "/name")?.as_str().check("tool name"))
        .collect::<TestResult<Vec<&str>>>()?;

    ensure_eq(&names.len(), &10, format!("{names:?}"))?;
    ensure(names.contains(&"finding_clear"), format!("{names:?}"))?;
    ensure(!names.contains(&"finding_sessions"), format!("{names:?}"))?;
    Ok(())
}

/// When the caller requests progress notifications (by passing
/// `_meta.progressToken` at the request level), the server must emit
/// monotonically non-decreasing `/notifications/progress` messages
/// (`0.0 <= p1 <= p2 <= ... <= 1.0`) with no `"id"` key on stdout *before*
/// the final tool-call response, on both a cold sync and a warm query.
#[test]
fn progress_notifications_flow_over_stdio() -> TestResult {
    let (_project, project_path) = ServerProcess::init_project()?;
    let mut server = ServerProcess::spawn()?;
    server.handshake()?;

    // Phase 1 — cold sync `report` *with* a progress token so sync-layer
    // phase notifications and lifecycle notifications both travel over stdio.
    let sync_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "report",
            "arguments": { "path": project_path },
            "_meta": { "progressToken": "pt-sync" },
        },
    });
    server.send(&sync_call.to_string())?;
    let (sync_response, sync_notifications) = server.recv_response(2)?;
    ensure(
        at(&sync_response, "/result")?.is_object(),
        format!("initial sync must succeed: {sync_response}"),
    )?;
    check_monotonic_progress(&sync_notifications, "pt-sync")?;

    // Phase 2 — warm `query` with a fresh progress token.
    let query_call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "query",
            "arguments": { "path": project_path, "sql": "SELECT 1" },
            "_meta": { "progressToken": "pt-q" },
        },
    });
    server.send(&query_call.to_string())?;
    let (response, notifications) = server.recv_response(3)?;

    ensure(
        at(&response, "/result")?.is_object(),
        format!("query must succeed: {response}"),
    )?;
    check_monotonic_progress(&notifications, "pt-q")?;
    Ok(())
}

/// Asserts `mechanic list` and `mechanic remove` responses plus the post-remove
/// `leads` counts for `"rs_dangerous_hook"`.
fn check_mechanic_list_and_remove(
    listed: &serde_json::Value,
    removed: &serde_json::Value,
    leads_after_remove: &serde_json::Value,
) -> TestResult {
    ensure_eq(
        at(listed, "/action")?,
        &"list",
        "mechanic list echoes action `list`",
    )?;
    ensure_eq(
        at(listed, "/id")?,
        &serde_json::Value::Null,
        "mechanic list sets id to null",
    )?;
    ensure_eq(
        at(listed, "/page/complete")?,
        &true,
        "mechanic list fits on one complete page",
    )?;
    ensure_eq(
        at(listed, "/mechanics/0/id")?,
        &"rs_dangerous_hook",
        "mechanic list orders session mechanic first",
    )?;
    ensure_eq(
        at(listed, "/mechanics/0/source")?,
        &"session",
        "mechanic list marks custom entry as session",
    )?;

    ensure_eq(
        at(removed, "/action")?,
        &"remove",
        "mechanic remove echoes action `remove`",
    )?;
    ensure_eq(
        at(removed, "/id")?,
        &"rs_dangerous_hook",
        "mechanic remove echoes removed id",
    )?;
    ensure_eq(
        at(removed, "/source")?,
        &"session",
        "mechanic remove reports session source",
    )?;
    let remaining_ids = at(removed, "/mechanics")?
        .as_array()
        .check("removed mechanics array")?
        .iter()
        .map(|entry| at(entry, "/id")?.as_str().check("mechanic entry id"))
        .collect::<TestResult<Vec<&str>>>()?;
    ensure(
        !remaining_ids.contains(&"rs_dangerous_hook"),
        format!("removed mechanic must disappear from catalogue: {remaining_ids:?}"),
    )?;
    let post_remove_leads = at(leads_after_remove, "/mechanic_counts")?
        .as_array()
        .check("leads mechanic_counts array")?;
    ensure(
        post_remove_leads
            .iter()
            .all(|entry| entry.get("mechanic") != Some(&json!("rs_dangerous_hook"))),
        format!("leads after remove must not count removed mechanic: {leads_after_remove}"),
    )
}

/// Full auditor-owned `mechanic` flow over real stdio (`add` -> `leads` ->
/// `list` -> `remove`), plus verifying `report.mechanics.note` is a string on
/// the first `report` and explicitly `null` on subsequent `report` calls.
#[test]
fn real_stdio_mechanic_add_is_counted_and_listed_first_by_leads() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;
    std::fs::write(
        project.path().join("src.rs"),
        b"pub fn run() {\n    dangerous_hook(); // dangerous_hook()\n    let _s = \"dangerous_hook()\";\n}\nfn dangerous_hook() {}\n",
    )
    .check("write Rust fixture file")?;
    let project_path = project.path().to_string_lossy().replace('\\', "/");
    let mut server = ServerProcess::spawn()?;
    server.handshake()?;

    let first_report = server.tool_json(2, "report", &json!({ "path": project_path }))?;
    let added = server.tool_json(
        3,
        "mechanic",
        &json!({
            "path": project_path,
            "action": "add",
            "id": "rs_dangerous_hook",
            "language": "rust",
            "query": "(call_expression function: (identifier) @callee (#eq? @callee \"dangerous_hook\"))",
            "note": "explicit calls to dangerous_hook",
        }),
    )?;
    let leads = server.tool_json(4, "leads", &json!({ "path": project_path }))?;
    let second_report = server.tool_json(5, "report", &json!({ "path": project_path }))?;
    let listed = server.tool_json(
        6,
        "mechanic",
        &json!({ "path": project_path, "action": "list" }),
    )?;
    let removed = server.tool_json(
        7,
        "mechanic",
        &json!({
            "path": project_path,
            "action": "remove",
            "id": "rs_dangerous_hook",
        }),
    )?;
    let leads_after_remove = server.tool_json(8, "leads", &json!({ "path": project_path }))?;

    ensure(
        at(&first_report, "/mechanics/note")?
            .as_str()
            .is_some_and(|note| note.contains("not a checklist")),
        format!("the first report carries the mechanics note: {first_report}"),
    )?;
    ensure_eq(
        at(&added, "/source")?,
        &"session",
        "added mechanic source is session",
    )?;
    ensure_eq(
        at(&added, "/first_count/page/total")?,
        &1,
        format!("first_count finds only the real call site, ignoring comment and string: {added}"),
    )?;
    ensure_eq(
        at(&added, "/first_count/rows/0/line")?,
        &2,
        format!("first_count reports line 2: {added}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/mechanic")?,
        &"rs_dangerous_hook",
        format!("session mechanic is listed first in leads: {leads}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/source")?,
        &"session",
        format!("leads marks the custom mechanic as session: {leads}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/count")?,
        &1,
        format!("leads counts 1 call site: {leads}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/covers")?,
        &json!(["rust"]),
        "leads covers reports rust",
    )?;
    // Strict key check: `/mechanics/note` must be present and equal to JSON `null`.
    ensure_eq(
        at(&second_report, "/mechanics/note")?,
        &serde_json::Value::Null,
        format!("second report must serialize mechanics.note as null: {second_report}"),
    )?;

    check_mechanic_list_and_remove(&listed, &removed, &leads_after_remove)
}

/// Live stdio lifecycle for `project_control`: `health` reports `disabled` on
/// an unenabled directory, `project_control on` sets up `.planning/README.md`
/// and `.planning/slugaudit/` and runs the initial import, `report` succeeds,
/// and `project_control off` deletes `.planning/slugaudit/` while preserving
/// `.planning/README.md` and returning the project to `project_not_enabled`.
#[test]
fn real_stdio_project_control_on_report_and_off_lifecycle() -> TestResult {
    let project = tempfile::tempdir().check("create unenabled project dir")?;
    std::fs::write(
        project.path().join("lib.rs"),
        b"pub fn answer() -> u64 { 42 }\n",
    )
    .check("write lib.rs")?;
    let project_path = project.path().to_string_lossy().replace('\\', "/");

    let mut server = ServerProcess::spawn()?;
    server.handshake()?;

    let health_before = server.tool_json(2, "health", &json!({ "path": project_path }))?;
    ensure_eq(
        at(&health_before, "/phase")?,
        &"disabled",
        "unenabled project reports phase disabled",
    )?;
    ensure(
        at(&health_before, "/remedy")?
            .as_str()
            .is_some_and(|remedy| remedy.contains("project_control")),
        format!("disabled health includes project_control remedy: {health_before}"),
    )?;

    let enabled = server.tool_json(
        3,
        "project_control",
        &json!({ "path": project_path, "action": "on", "profile": "lean" }),
    )?;
    ensure_eq(
        at(&enabled, "/status")?,
        &"enabled",
        "project_control on reports status enabled",
    )?;
    ensure(
        at(&enabled, "/import/added")?
            .as_u64()
            .is_some_and(|added| added >= 1),
        format!("initial import adds discovered files: {enabled}"),
    )?;
    ensure_eq(
        at(&enabled, "/gitignore/status")?,
        &"added",
        "project_control on adds .planning/slugaudit/ to .gitignore",
    )?;
    ensure(
        project
            .path()
            .join(".planning/slugaudit/project.db")
            .is_file(),
        "project_control on creates .planning/slugaudit/project.db",
    )?;
    ensure(
        project.path().join(".planning/README.md").is_file(),
        "project_control on creates .planning/README.md via slugplan",
    )?;

    let report = server.tool_json(4, "report", &json!({ "path": project_path }))?;
    ensure(
        at(&report, "/revision_id")?.is_string(),
        "report returns a committed revision_id on the enabled project",
    )?;

    let disabled = server.tool_json(
        5,
        "project_control",
        &json!({ "path": project_path, "action": "off" }),
    )?;
    ensure_eq(
        at(&disabled, "/status")?,
        &"disabled",
        "project_control off reports status disabled",
    )?;
    ensure(
        !project.path().join(".planning/slugaudit").exists(),
        "project_control off deletes .planning/slugaudit/",
    )?;
    ensure(
        project.path().join(".planning/README.md").is_file(),
        "project_control off leaves .planning/README.md untouched",
    )?;

    let after_off = server.tool_response(6, "report", &json!({ "path": project_path }))?;
    ensure_eq(
        at(&after_off, "/error/data/code")?,
        &"project_not_enabled",
        format!("report after project_control off must return project_not_enabled: {after_off}"),
    )?;
    Ok(())
}

/// Asserts `health(verbose: true)` fields before any paged result is created
/// (`detail.database`, `detail.server`, and `detail.result_store == null`).
fn check_verbose_health_before_paging(health_before: &serde_json::Value) -> TestResult {
    ensure_eq(
        at(health_before, "/phase")?,
        &"steady_state",
        "synced project reports steady_state phase",
    )?;
    ensure_eq(
        at(health_before, "/remedy")?,
        &serde_json::Value::Null,
        "steady_state project has null remedy",
    )?;
    ensure(
        at(health_before, "/detail/database/parser_pack_version")?.is_string(),
        "verbose health carries detail.database.parser_pack_version",
    )?;
    ensure_eq(
        at(health_before, "/detail/database/skipped_file_count")?,
        &0,
        "verbose health reports 0 skipped files",
    )?;
    ensure(
        at(health_before, "/detail/server/tool_call_count")?
            .as_u64()
            .is_some_and(|calls| calls >= 1),
        "verbose health detail.server.tool_call_count reflects prior calls",
    )?;
    ensure_eq(
        at(health_before, "/detail/server/tool_call_error_count")?,
        &0,
        "verbose health detail.server.tool_call_error_count is 0",
    )?;
    ensure(
        at(health_before, "/detail/server/tool_call_total_ms")?.is_u64(),
        "verbose health detail.server.tool_call_total_ms is a u64",
    )?;
    ensure(
        at(health_before, "/detail/server/last_sync_unix_seconds")?
            .as_i64()
            .is_some_and(|ts| ts > 0),
        "verbose health detail.server.last_sync_unix_seconds is stamped",
    )?;
    ensure(
        at(health_before, "/detail/server/last_sync_duration_ms")?.is_u64(),
        "verbose health detail.server.last_sync_duration_ms is a u64",
    )?;
    ensure_eq(
        at(health_before, "/detail/result_store")?,
        &serde_json::Value::Null,
        "detail.result_store is null before the first paged result",
    )
}

/// Pages a 12-file `query` to completion via cursor-only continuation calls and
/// returns `(next_request_id, collected_paths)`.
fn collect_paged_query_paths(
    server: &mut ServerProcess,
    project_path: &str,
    start_id: u64,
) -> TestResult<(u64, Vec<String>)> {
    let first_page = server.tool_json(
        start_id,
        "query",
        &json!({
            "path": project_path,
            "sql": "SELECT path, content FROM files WHERE language = 'rust' ORDER BY path",
            "page_bytes": 350,
        }),
    )?;
    ensure_eq(
        at(&first_page, "/page/complete")?,
        &false,
        "first query page must be incomplete at 350 page_bytes",
    )?;
    ensure_eq(
        at(&first_page, "/page/limited_by")?,
        &"page_bytes",
        "first query page is cut by page_bytes",
    )?;
    ensure_eq(
        at(&first_page, "/page/total")?,
        &12,
        "query page.total counts all 12 Rust files",
    )?;

    let mut collected_paths = Vec::new();
    for row in at(&first_page, "/rows")?.as_array().check("page 1 rows")? {
        collected_paths.push(at(row, "/path")?.as_str().check("row path")?.to_owned());
    }
    let mut next_cursor = Some(
        at(&first_page, "/page/cursor")?
            .as_str()
            .check("page 1 carries a continuation cursor")?
            .to_owned(),
    );
    let mut request_id = start_id
        .checked_add(1)
        .check("request id counter overflowed")?;
    let mut page_count = 1_usize;

    while let Some(cursor) = next_cursor {
        let continuation = server.tool_json(request_id, "query", &json!({ "cursor": cursor }))?;
        request_id = request_id
            .checked_add(1)
            .check("request id counter overflowed")?;
        page_count = page_count.checked_add(1).check("page count overflowed")?;
        ensure(
            page_count <= 20,
            "cursor continuation must terminate within 12 rows",
        )?;

        for row in at(&continuation, "/rows")?
            .as_array()
            .check("continuation rows")?
        {
            collected_paths.push(at(row, "/path")?.as_str().check("row path")?.to_owned());
        }
        let complete = at(&continuation, "/page/complete")?
            .as_bool()
            .check("page.complete is a boolean")?;
        let page_obj = at(&continuation, "/page")?
            .as_object()
            .check("page is an object")?;
        if complete {
            ensure(
                !page_obj.contains_key("cursor"),
                format!("completed final page must omit cursor: {continuation}"),
            )?;
            next_cursor = None;
        } else {
            next_cursor = Some(
                at(&continuation, "/page/cursor")?
                    .as_str()
                    .check("incomplete continuation page carries cursor")?
                    .to_owned(),
            );
        }
    }

    ensure(
        page_count >= 2,
        format!("expected multi-page query continuation, got {page_count} page(s)"),
    )?;
    Ok((request_id, collected_paths))
}

/// Asserts typed `detail.result_store` fields on `health(verbose: true)` after
/// a paged result has been read to completion.
fn check_verbose_health_after_paging(health_after: &serde_json::Value) -> TestResult {
    ensure(
        at(health_after, "/detail/result_store/budget_bytes")?
            .as_u64()
            .is_some_and(|budget| budget > 0),
        "detail.result_store.budget_bytes is positive once opened",
    )?;
    ensure(
        at(health_after, "/detail/result_store/admitted_bytes")?.is_u64(),
        "detail.result_store.admitted_bytes is a u64",
    )?;
    ensure(
        at(health_after, "/detail/result_store/active/results")?.is_u64(),
        "detail.result_store.active.results is a u64",
    )?;
    ensure(
        at(health_after, "/detail/result_store/idle/results")?.is_u64(),
        "detail.result_store.idle.results is a u64",
    )?;
    ensure(
        at(health_after, "/detail/result_store/delivered/results")?
            .as_u64()
            .is_some_and(|delivered| delivered >= 1),
        "completed query result transitions into delivered class",
    )?;
    ensure(
        at(health_after, "/detail/result_store/sweep/removed_files")?.is_u64(),
        "detail.result_store.sweep.removed_files is a u64",
    )
}

/// Live stdio coverage for `query` cursor continuation (`page 1` -> `page 2` ->
/// completion with cursor-only calls) and `health` with `verbose: true`
/// verifying typed `detail.database`, `detail.server`, and `detail.result_store`
/// before and after paging.
#[test]
fn real_stdio_query_cursor_continuation_and_verbose_health_detail() -> TestResult {
    let project = tempfile::tempdir().check("create multi-file project dir")?;
    std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;

    let mut expected_paths = Vec::new();
    for idx in 0..12_u32 {
        let filename = format!("mod_{idx:02}.rs");
        let content = format!(
            "//! Module {idx:02} padding text so serialized rows exceed a small page budget.\npub fn item_{idx:02}() -> u32 {{ {idx} }}\n"
        );
        std::fs::write(project.path().join(&filename), content).check("write fixture module")?;
        expected_paths.push(filename);
    }
    let project_path = project.path().to_string_lossy().replace('\\', "/");

    let mut server = ServerProcess::spawn()?;
    server.handshake()?;
    server.tool_json(2, "report", &json!({ "path": project_path }))?;

    let health_before = server.tool_json(
        3,
        "health",
        &json!({ "path": project_path, "verbose": true }),
    )?;
    check_verbose_health_before_paging(&health_before)?;

    let (next_id, collected_paths) = collect_paged_query_paths(&mut server, &project_path, 4)?;
    ensure_eq(
        &collected_paths,
        &expected_paths,
        "cursor continuation reconstructs the exact ordered row sequence without duplicates or gaps",
    )?;

    let health_after = server.tool_json(
        next_id,
        "health",
        &json!({ "path": project_path, "verbose": true }),
    )?;
    check_verbose_health_after_paging(&health_after)
}

/// Live stdio coverage for `resources/list` and `resources/read` (both valid
/// embedded URIs and `resource_not_found` on an unknown URI).
#[test]
fn real_stdio_resources_list_and_read_serve_embedded_catalogue() -> TestResult {
    let mut server = ServerProcess::spawn()?;
    server.handshake()?;

    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}"#)?;
    let (list_response, _) = server.recv_response(2)?;
    let uris = at(&list_response, "/result/resources")?
        .as_array()
        .check("resources array")?
        .iter()
        .map(|entry| at(entry, "/uri")?.as_str().check("resource uri"))
        .collect::<TestResult<Vec<&str>>>()?;
    ensure_eq(
        &uris.len(),
        &10,
        format!("expected 10 embedded resources, got {uris:?}"),
    )?;
    ensure(
        uris.contains(&"slugaudit://schema.sql"),
        format!("resources/list must advertise slugaudit://schema.sql: {uris:?}"),
    )?;
    ensure(
        uris.contains(&"slugaudit://queries/README.md"),
        format!("resources/list must advertise slugaudit://queries/README.md: {uris:?}"),
    )?;

    server.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"slugaudit://schema.sql"}}"#,
    )?;
    let (read_response, _) = server.recv_response(3)?;
    ensure_eq(
        at(&read_response, "/result/contents/0/uri")?,
        &"slugaudit://schema.sql",
        "resources/read echoes the requested URI",
    )?;
    ensure_eq(
        at(&read_response, "/result/contents/0/mimeType")?,
        &"application/sql",
        "schema.sql carries application/sql mimeType",
    )?;
    ensure(
        at(&read_response, "/result/contents/0/text")?
            .as_str()
            .is_some_and(|sql| sql.contains("CREATE TABLE IF NOT EXISTS files")),
        format!("schema.sql text contains canonical table DDL: {read_response}"),
    )?;

    server.send(
        r#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"slugaudit://does-not-exist"}}"#,
    )?;
    let (missing_response, _) = server.recv_response(4)?;
    ensure_eq(
        at(&missing_response, "/error/code")?,
        &-32002,
        format!("unknown resource URI must return -32002 resource_not_found: {missing_response}"),
    )?;
    ensure(
        at(&missing_response, "/error/message")?
            .as_str()
            .is_some_and(|msg| msg.contains("unknown resource")),
        format!("resource_not_found names unknown resource: {missing_response}"),
    )?;
    Ok(())
}

/// Cold subprocess startup and MCP `initialize` JSON-RPC response must complete
/// in under one second.
#[test]
fn initialize_responds_under_one_second() -> TestResult {
    let started = Instant::now();
    let mut server = ServerProcess::spawn()?;
    server.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"cold-start-check","version":"0.0.1"}}}"#,
    )?;
    let (response, _) = server.recv_response(1)?;
    let elapsed = started.elapsed();

    ensure_eq(
        at(&response, "/result/serverInfo/name")?,
        &"slugaudit",
        "initialize response identifies the slugaudit MCP server",
    )?;
    ensure(
        elapsed < Duration::from_secs(1),
        format!("cold-start MCP initialize took {elapsed:?}, exceeding the 1s ceiling"),
    )?;
    Ok(())
}
