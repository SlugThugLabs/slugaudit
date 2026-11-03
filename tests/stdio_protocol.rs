#![forbid(unsafe_code)]

//! Proves the compiled server speaks real MCP over real stdio: the other
//! tests in this repo call Rust functions directly and never touch the
//! actual protocol. This spawns the real binary as a subprocess.

use serde_json::json;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

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
        let response: serde_json::Value = serde_json::from_str(&self.recv_stdout_line()?)
            .check(&format!("{name} response is valid JSON"))?;
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

    /// Initialize the server and return a project path string suitable
    /// for embedding in tool-call JSON.  Writes a single `lib.rs` fixture
    /// and handles the init/initialized handshake.
    fn init_project() -> TestResult<(tempfile::TempDir, String)> {
        let project = tempfile::tempdir().check("project dir")?;
        std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
            .check("activate project")?;
        write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
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

/// Workflow act 2: the finding bound to `lib.rs` must read stale once the
/// source changes, across a real sync on the live `server`.
fn check_finding_reads_stale_after_the_source_changes(
    server: &mut ServerProcess,
    project: &tempfile::TempDir,
    project_path: &str,
) -> TestResult {
    // --- Workflow act 2: the finding must read stale the moment the source
    // it was bound to changes, across a real sync on a live server. ---
    std::fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .check("modify source")?;
    // The next tool call hashes the tree, sees the change, and publishes
    // revision 2 (the finding reads stale). Poll the revision count
    // instead of sleeping a fixed amount, so a slow CI machine can't turn
    // a correct test into a flaky one.
    let mut revision_count: i64 = 1;
    for _ in 0..10 {
        let sync_result = server.tool_json(
            7,
            "query",
            &json!({ "path": project_path, "sql": "SELECT count(*) AS n FROM revisions" }),
        )?;
        revision_count = at(&sync_result, "/rows/0/n")?
            .as_i64()
            .check("revision count is a number")?;
        if revision_count == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    ensure_eq(
        &revision_count,
        &2,
        "the modification must publish a second revision",
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
    // Structured tracing (src/server.rs's `run_blocking`, src/tools/*.rs)
    // must actually reach stderr with the fields it claims to record, and
    // — the point of this whole test file — none of that can leak onto
    // stdout, which every `recv_stdout_line` + `serde_json::from_str`
    // call above already implicitly guarantees (any stray non-JSON text on
    // a stdout line would fail parsing), but assert it explicitly
    // for the tracing content specifically.
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
        "report[\"file_count\"] == 1",
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
        "languages.iter().any(|entry| entry[\"language\"] == \"rust\")",
    )?;

    let query_result = server.tool_json(
        3,
        "query",
        &json!({ "path": project_path, "sql": "SELECT path, language FROM files" }),
    )?;
    ensure_eq(
        at(&query_result, "/rows/0/path")?,
        &"lib.rs",
        "query_result[\"rows\"][0][\"path\"] == \"lib.rs\"",
    )?;
    ensure_eq(
        at(&query_result, "/rows/0/language")?,
        &"rust",
        "query_result[\"rows\"][0][\"language\"] == \"rust\"",
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
        "structure_result[\"matches\"][0][\"text\"] == \"a\"",
    )?;
    ensure_eq(
        at(&structure_result, "/matches/0/capture_name")?,
        &"name",
        "structure_result[\"matches\"][0][\"capture_name\"] == \"name\"",
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
        "finding_result[\"status\"] == \"current\"",
    )?;
    ensure(
        at(&finding_result, "/id")?
            .as_i64()
            .is_some_and(|id| id > 0),
        "finding_result[\"id\"].as_i64().is_some_and(|id| id > 0)",
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
        let call = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"report","arguments":{{"path":"{project_path}"}}}}}}"#
        );
        server.send(&call)?;
        let response: serde_json::Value =
            serde_json::from_str(&server.recv_stdout_line()?).check("report response")?;
        let content = at(&response, "/result/content/0/text")?
            .as_str()
            .check("report text")?;
        let report: serde_json::Value = serde_json::from_str(content).check("report JSON")?;
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
        "read[\"findings\"].as_array().map(Vec::len) == Some(1)",
    )?;
    let cleared = first.tool_json(4, "finding_clear", &json!({ "path": project_path }))?;
    ensure_eq(at(&cleared, "/removed")?, &1, "cleared[\"removed\"] == 1")?;
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
        "titles(&mut first, 3)? == [\"from first\"]",
    )?;
    ensure_eq(
        &titles(&mut second, 4)?,
        &["from second"],
        "titles(&mut second, 4)? == [\"from second\"]",
    )?;
    let cleared = second.tool_json(5, "finding_clear", &json!({ "path": project_path }))?;
    ensure_eq(at(&cleared, "/removed")?, &1, "cleared[\"removed\"] == 1")?;
    ensure_eq(
        &titles(&mut first, 4)?,
        &["from first"],
        "titles(&mut first, 4)? == [\"from first\"]",
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
/// `/notifications/progress` messages on stdout *before* the final
/// tool-call response.  This exercises the path from `progress_target` →
/// `McpProgressSink` → `notify_progress` that every other test bypasses
/// by calling Rust functions directly.
///
/// We sync the project first (without a progress token) so the follow-up
/// query is a fast, no-publish call — the progress pipeline still fires
/// regardless.
#[test]
fn progress_notifications_flow_over_stdio() -> TestResult {
    let (_project, project_path) = ServerProcess::init_project()?;
    let mut server = ServerProcess::spawn()?;
    server.handshake()?;

    // Phase 1 — sync without progress: drain any stdout, verify success.
    let sync_call = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"report","arguments":{{"path":"{project_path}"}}}}}}"#
    );
    server.send(&sync_call)?;
    let (sync_response, _) = server.recv_response(2)?;
    ensure(
        at(&sync_response, "/result")?.is_object(),
        format!("initial sync must succeed: {sync_response}"),
    )?;

    // Phase 2 — query *with* a progress token inside `params._meta`
    // (the MCP tool-call convention). The project is already synced;
    // this should be a fast call, but the progress notifications must
    // still arrive.
    let query_call = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"query","arguments":{{"path":"{project_path}","sql":"SELECT 1"}},"_meta":{{"progressToken":"pt-q"}}}}}}"#
    );
    server.send(&query_call)?;
    let (response, notifications) = server.recv_response(3)?;

    ensure(
        at(&response, "/result")?.is_object(),
        format!("query must succeed: {response}"),
    )?;

    ensure(
        !notifications.is_empty(),
        "at least one progress notification must arrive when progressToken is set",
    )?;

    for (i, notification) in notifications.iter().enumerate() {
        ensure(
            notification
                .get("id")
                .is_none_or(serde_json::Value::is_null),
            format!("progress notification {i} must have no 'id': {notification}"),
        )?;
        ensure_eq(
            at(notification, "/method")?,
            &"notifications/progress",
            format!("stdout line {i} must be a progress notification: {notification}"),
        )?;
        ensure_eq(
            at(notification, "/params/progressToken")?,
            &"pt-q",
            format!("progress token must round-trip: {notification}"),
        )?;
        at(notification, "/params/progress")?
            .as_f64()
            .check("`notification['params']['progress'].as_f64()` is present")?;
    }

    // The last notification must signal completion at 1.0.
    let last = notifications
        .last()
        .check("at least one progress notification")?;
    ensure_eq(
        at(last, "/params/progress")?,
        &1.0,
        format!("final progress notification must report completion at 1.0: {last}"),
    )?;
    Ok(())
}

/// The auditor-owned mechanics flow over real stdio, in one live server:
/// the first `report` carries the mechanics note, `mechanic add` returns the
/// new entry's first count, `leads` lists it first as a session entry, and a
/// second `report` no longer carries the note.
#[test]
fn real_stdio_mechanic_add_is_counted_and_listed_first_by_leads() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;
    write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    std::fs::write(
        project.path().join("app.py"),
        b"def run(code):\n    eval(code)  # eval(comment)\n    return \"eval(text)\"\n",
    )
    .check("write fixture file")?;
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
            "id": "py_eval",
            "language": "python",
            "query": "(call function: (identifier) @callee (#eq? @callee \"eval\"))",
            "note": "dynamic code execution",
        }),
    )?;
    let leads = server.tool_json(4, "leads", &json!({ "path": project_path }))?;
    let second_report = server.tool_json(5, "report", &json!({ "path": project_path }))?;

    ensure(
        at(&first_report, "/mechanics/note")?
            .as_str()
            .is_some_and(|note| note.contains("not a checklist")),
        format!("the first report carries the note: {first_report}"),
    )?;
    ensure_eq(
        at(&added, "/source")?,
        &"session",
        "added[\"source\"] == \"session\"",
    )?;
    ensure_eq(
        at(&added, "/first_count/page/total")?,
        &1,
        format!("{added}"),
    )?;
    ensure_eq(
        at(&added, "/first_count/rows/0/line")?,
        &2,
        format!("{added}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/mechanic")?,
        &"py_eval",
        format!("{leads}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/source")?,
        &"session",
        format!("{leads}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/count")?,
        &1,
        format!("{leads}"),
    )?;
    ensure_eq(
        at(&leads, "/mechanic_counts/0/covers")?,
        &json!(["python"]),
        "leads[\"mechanic_counts\"][0][\"covers\"] == json!([\"python\"])",
    )?;
    ensure(
        second_report
            .pointer("/mechanics/note")
            .is_none_or(serde_json::Value::is_null),
        format!("only the first report carries the note: {second_report}"),
    )?;
    Ok(())
}

/// Creates the empty owner-only `project.db` that marks a project as active.
fn write_database_marker(activation: &std::path::Path) -> TestResult {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(activation.join("project.db"))
        .check("create database marker")?;
    Ok(())
}
