//! Proves the compiled server speaks real MCP over real stdio: every prior
//! test in this repo calls Rust functions directly and never touches the
//! actual protocol. This spawns the real binary as a subprocess.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct ServerProcess {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout_lines: mpsc::Receiver<String>,
    stderr_lines: mpsc::Receiver<String>,
}

impl ServerProcess {
    fn spawn() -> Self {
        let binary = env!("CARGO_BIN_EXE_slugaudit");
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn slugaudit");

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");

        let (stdout_tx, stdout_lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if stdout_tx.send(line).is_err() {
                    return;
                }
            }
        });
        let (stderr_tx, stderr_lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if stderr_tx.send(line).is_err() {
                    return;
                }
            }
        });

        Self {
            child,
            stdin: Some(stdin),
            stdout_lines,
            stderr_lines,
        }
    }

    fn send(&mut self, json: &str) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        writeln!(stdin, "{json}").expect("write request line");
        stdin.flush().expect("flush request");
    }

    fn recv_stdout_line(&self) -> String {
        self.stdout_lines
            .recv_timeout(Duration::from_secs(10))
            .expect("a stdout line within timeout")
    }

    /// Reads stdout lines until a JSON-RPC response with the given `id`
    /// appears. Returns the response line and any `/notifications/progress`
    /// lines that arrived before it — progress notifications have no `id`,
    /// so they cannot be the requested response, and this function drains
    /// them instead of silently dropping them between `recv_stdout_line`
    /// calls.
    fn recv_response(&self, expected_id: u64) -> (serde_json::Value, Vec<serde_json::Value>) {
        let mut notifications = Vec::new();
        loop {
            let line = self.recv_stdout_line();
            let parsed: serde_json::Value =
                serde_json::from_str(&line).expect("stdout line is valid JSON");
            if parsed["id"] == expected_id {
                return (parsed, notifications);
            }
            // A line without an `id` matching the expected one is a
            // notification — keep it and wait for the real response.
            notifications.push(parsed);
        }
    }

    fn has_stderr_output(&self) -> bool {
        self.stderr_lines
            .recv_timeout(Duration::from_secs(2))
            .is_ok()
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
    fn init_project() -> (tempfile::TempDir, String) {
        let project = tempfile::tempdir().expect("project dir");
        std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
            .expect("activate project");
        std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n")
            .expect("write fixture file");
        let path = project.path().to_string_lossy().replace('\\', "/");
        (project, path)
    }

    fn handshake(&mut self) {
        self.send(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client","version":"0.0.1"}}}"#,
        );
        let _: serde_json::Value =
            serde_json::from_str(&self.recv_stdout_line()).expect("init response");
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
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
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// One continuous live server process exercising a real sequence of calls
// is the point of this test — splitting it into several #[test] fns would
// each need to re-spawn and re-initialize a fresh process, losing exactly
// the "state carries correctly across a real session" property this test
// exists to prove.
#[allow(clippy::too_many_lines)]
#[test]
fn real_stdio_handshake_and_tool_call_stay_protocol_pure() {
    let (_project, project_path) = ServerProcess::init_project();

    let mut server = ServerProcess::spawn();
    server.handshake();

    let call = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"report","arguments":{{"path":"{project_path}"}}}}}}"#
    );
    server.send(&call);

    let call_response: serde_json::Value =
        serde_json::from_str(&server.recv_stdout_line()).expect("tool call response is valid JSON");
    assert_eq!(call_response["id"], 2);
    let content = call_response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result has text content");
    let report: serde_json::Value =
        serde_json::from_str(content).expect("tool result text is JSON");
    assert_eq!(report["file_count"], 1);
    assert!(
        report["languages"]
            .as_array()
            .expect("languages array")
            .iter()
            .any(|entry| entry["language"] == "rust")
    );

    let query_call = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"query","arguments":{{"path":"{project_path}","sql":"SELECT path, language FROM files"}}}}}}"#
    );
    server.send(&query_call);
    let query_response: serde_json::Value =
        serde_json::from_str(&server.recv_stdout_line()).expect("query response is valid JSON");
    assert_eq!(query_response["id"], 3);
    let query_content = query_response["result"]["content"][0]["text"]
        .as_str()
        .expect("query result has text content");
    let query_result: serde_json::Value =
        serde_json::from_str(query_content).expect("query result text is JSON");
    assert_eq!(query_result["rows"][0]["path"], "lib.rs");
    assert_eq!(query_result["rows"][0]["language"], "rust");

    let write_attempt = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"query","arguments":{{"path":"{project_path}","sql":"DELETE FROM files"}}}}}}"#
    );
    server.send(&write_attempt);
    let write_response: serde_json::Value = serde_json::from_str(&server.recv_stdout_line())
        .expect("write-attempt response is valid JSON");
    assert_eq!(write_response["id"], 4);
    assert!(
        write_response["error"]["code"].is_number(),
        "a write attempt through query must come back as a typed protocol error, not succeed: {write_response}"
    );

    let structure_call = format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"structure","arguments":{{"path":"{project_path}","file":"lib.rs","query":"(function_item name: (identifier) @name)"}}}}}}"#
    );
    server.send(&structure_call);
    let structure_response: serde_json::Value =
        serde_json::from_str(&server.recv_stdout_line()).expect("structure response is valid JSON");
    assert_eq!(structure_response["id"], 5);
    let structure_content = structure_response["result"]["content"][0]["text"]
        .as_str()
        .expect("structure result has text content");
    let structure_result: serde_json::Value =
        serde_json::from_str(structure_content).expect("structure result text is JSON");
    assert_eq!(structure_result["matches"][0]["text"], "a");
    assert_eq!(structure_result["matches"][0]["capture_name"], "name");

    let finding_call = format!(
        r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"finding","arguments":{{"path":"{project_path}","file":"lib.rs","line_start":1,"line_end":1,"severity":"low","category":"style","title":"t","description":"d"}}}}}}"#
    );
    server.send(&finding_call);
    let finding_response: serde_json::Value =
        serde_json::from_str(&server.recv_stdout_line()).expect("finding response is valid JSON");
    assert_eq!(finding_response["id"], 6);
    let finding_content = finding_response["result"]["content"][0]["text"]
        .as_str()
        .expect("finding result has text content");
    let finding_result: serde_json::Value =
        serde_json::from_str(finding_content).expect("finding result text is JSON");
    assert_eq!(finding_result["status"], "current");
    assert!(finding_result["id"].as_i64().is_some_and(|id| id > 0));

    // --- Workflow act 2: the finding must go stale the moment the source
    // it was bound to changes, across a real sync on a live server. ---
    std::fs::write(
        _project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify source");
    // The server's watcher delivers modify events asynchronously; poll the
    // revision count instead of sleeping a fixed amount, so a slow CI
    // machine can't turn a correct test into a flaky one. Each call that
    // sees no pending event is a no-op sync; once the event lands, the
    // reconcile publishes revision 2 (and invalidates the finding).
    let mut revision_count: i64 = 1;
    for _ in 0..10 {
        let sync_call = format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"query","arguments":{{"path":"{project_path}","sql":"SELECT count(*) AS n FROM revisions"}}}}}}"#
        );
        server.send(&sync_call);
        let sync_response: serde_json::Value =
            serde_json::from_str(&server.recv_stdout_line()).expect("sync response is valid JSON");
        assert_eq!(sync_response["id"], 7);
        let sync_content = sync_response["result"]["content"][0]["text"]
            .as_str()
            .expect("sync result has text content");
        let sync_result: serde_json::Value =
            serde_json::from_str(sync_content).expect("sync result text is JSON");
        revision_count = sync_result["rows"][0]["n"]
            .as_i64()
            .expect("revision count is a number");
        if revision_count == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(
        revision_count, 2,
        "the modification must publish a second revision"
    );

    let stale_call = format!(
        r#"{{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{{"name":"query","arguments":{{"path":"{project_path}","sql":"SELECT status FROM findings"}}}}}}"#
    );
    server.send(&stale_call);
    let stale_response: serde_json::Value =
        serde_json::from_str(&server.recv_stdout_line()).expect("stale response is valid JSON");
    assert_eq!(stale_response["id"], 8);
    let stale_content = stale_response["result"]["content"][0]["text"]
        .as_str()
        .expect("stale result has text content");
    let stale_result: serde_json::Value =
        serde_json::from_str(stale_content).expect("stale result text is JSON");
    assert_eq!(
        stale_result["rows"][0]["status"], "stale",
        "the finding must flip to stale once its source changed"
    );

    assert!(
        server.has_stderr_output(),
        "startup diagnostics should reach stderr"
    );

    // Structured tracing (src/server.rs's `run_blocking`, src/tools/*.rs)
    // must actually reach stderr with the fields it claims to record, and
    // — the point of this whole test file — none of that can leak onto
    // stdout, which every prior `recv_stdout_line` + `serde_json::from_str`
    // call above already implicitly guarantees (any stray non-JSON text on
    // a stdout line would have failed parsing), but assert it explicitly
    // for the tracing content specifically since that's new since this
    // test was first written.
    let stderr = server.stderr_snapshot();
    assert!(
        stderr.contains("tool_call") && stderr.contains(r#"tool="report""#),
        "expected a report tool_call span in stderr, got:\n{stderr}"
    );
    assert!(
        stderr.contains("report built") && stderr.contains("revision_id"),
        "expected report's per-tool tracing fields in stderr, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("\"jsonrpc\""),
        "stderr must never carry JSON-RPC protocol content: {stderr}"
    );
}

/// Restart behavior (release-gate item): after the server process exits, a
/// fresh server must serve the project's last revision from the persisted
/// database — the database is the source of truth across restarts, and
/// freshness re-verification still converges on the same revision for an
/// unchanged project.
#[test]
fn restart_serves_the_same_revision_from_disk() {
    let (_project, project_path) = ServerProcess::init_project();

    let report_revision = |server: &mut ServerProcess| -> String {
        server.handshake();
        let call = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"report","arguments":{{"path":"{project_path}"}}}}}}"#
        );
        server.send(&call);
        let response: serde_json::Value =
            serde_json::from_str(&server.recv_stdout_line()).expect("report response");
        let content = response["result"]["content"][0]["text"]
            .as_str()
            .expect("report text");
        let report: serde_json::Value = serde_json::from_str(content).expect("report JSON");
        report["revision_id"]
            .as_str()
            .expect("revision id")
            .to_owned()
    };

    let mut first = ServerProcess::spawn();
    let first_revision = report_revision(&mut first);
    drop(first); // close stdin → graceful shutdown

    let mut second = ServerProcess::spawn();
    let second_revision = report_revision(&mut second);
    assert_eq!(
        second_revision, first_revision,
        "a fresh server must serve the same revision from the persisted database"
    );
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
fn progress_notifications_flow_over_stdio() {
    let (_project, project_path) = ServerProcess::init_project();
    let mut server = ServerProcess::spawn();
    server.handshake();

    // Phase 1 — sync without progress: drain any stdout, verify success.
    let sync_call = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"report","arguments":{{"path":"{project_path}"}}}}}}"#
    );
    server.send(&sync_call);
    let (sync_response, _) = server.recv_response(2);
    assert!(
        sync_response["result"].is_object(),
        "initial sync must succeed: {sync_response}"
    );

    // Phase 2 — query *with* a progress token inside `params._meta`
    // (the MCP tool-call convention). The project is already synced;
    // this should be a fast call, but the progress notifications must
    // still arrive.
    let query_call = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"query","arguments":{{"path":"{project_path}","sql":"SELECT 1"}},"_meta":{{"progressToken":"pt-q"}}}}}}"#
    );
    server.send(&query_call);
    let (response, notifications) = server.recv_response(3);

    assert!(
        response["result"].is_object(),
        "query must succeed: {response}"
    );

    assert!(
        !notifications.is_empty(),
        "at least one progress notification must arrive when progressToken is set"
    );

    for (i, notification) in notifications.iter().enumerate() {
        assert!(
            notification["id"].is_null(),
            "progress notification {i} must have no 'id': {notification}"
        );
        assert_eq!(
            notification["method"], "notifications/progress",
            "stdout line {i} must be a progress notification: {notification}"
        );
        assert_eq!(
            notification["params"]["progressToken"], "pt-q",
            "progress token must round-trip: {notification}"
        );
        assert!(
            notification["params"]["progress"].as_f64().is_some(),
            "progress must be a number: {notification}"
        );
    }

    // The last notification must signal completion at 1.0.
    let last = notifications
        .last()
        .expect("at least one progress notification");
    assert_eq!(
        last["params"]["progress"], 1.0,
        "final progress notification must report completion at 1.0: {last}"
    );
}
