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
        let binary = env!("CARGO_BIN_EXE_slugaudit-mcp-rust");
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn slugaudit-mcp-rust");

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

    fn has_stderr_output(&self) -> bool {
        self.stderr_lines
            .recv_timeout(Duration::from_secs(2))
            .is_ok()
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

#[test]
fn real_stdio_handshake_and_tool_call_stay_protocol_pure() {
    let project = tempfile::tempdir().expect("project dir");
    std::fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write fixture file");

    let mut server = ServerProcess::spawn();

    server.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client","version":"0.0.1"}}}"#,
    );
    let init_response: serde_json::Value = serde_json::from_str(&server.recv_stdout_line())
        .expect("initialize response is valid JSON");
    assert_eq!(init_response["id"], 1);
    assert!(init_response["result"]["serverInfo"].is_object());
    assert!(
        init_response["result"]["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("SlugAudit")),
        "initialize result should carry SlugAudit's instructions"
    );

    server.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let project_path = project.path().to_string_lossy().replace('\\', "/");
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

    assert!(
        server.has_stderr_output(),
        "startup diagnostics should reach stderr"
    );
}
