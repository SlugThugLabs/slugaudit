#![forbid(unsafe_code)]

//! Process-level exit-code contract of the `slugaudit` binary: usage errors
//! exit 2, a clean stop (stdin closed before `initialize`, SIGTERM) exits 0,
//! and no error is printed in Rust `Debug` form.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn slugaudit(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_slugaudit"));
    command.args(args);
    command
}

fn run(args: &[&str]) -> Output {
    slugaudit(args)
        .stdin(Stdio::null())
        .output()
        .expect("run slugaudit")
}

#[test]
fn cli_exit_unknown_flag_exits_2_with_a_display_message() {
    let output = run(&["--bogus"]);
    let stderr = String::from_utf8(output.stderr).expect("utf-8 stderr");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.starts_with("Error: unknown command \"--bogus\""),
        "{stderr}"
    );
}

#[test]
fn cli_exit_lowercase_v_prints_the_version_and_exits_0() {
    let output = run(&["-v"]);
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout, format!("slugaudit {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_exit_connect_with_an_extra_argument_exits_2() {
    let output = run(&["connect", "claude", "extra"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("utf-8 stderr");
    assert!(stderr.contains("unexpected argument \"extra\""), "{stderr}");
}

#[test]
fn cli_exit_stdin_closed_before_initialize_exits_0_with_one_clean_line() {
    let output = run(&[]);
    let stderr = String::from_utf8(output.stderr).expect("utf-8 stderr");
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains("ConnectionClosed("), "{stderr}");
}

#[test]
fn cli_exit_sigterm_to_a_serving_process_exits_0() {
    let mut child = slugaudit(&[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn slugaudit");
    // Like a real host: complete the handshake and keep stdin open until
    // after the exit assertion, so the stdin reader thread stays parked.
    let mut stdin = child.stdin.take().expect("stdin pipe");
    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    writeln!(stdin, "{initialize}").expect("write initialize");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout pipe"));
    let mut reply = String::new();
    stdout.read_line(&mut reply).expect("read initialize reply");
    assert!(reply.contains("\"result\""), "{reply}");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .expect("write initialized");
    stdin.flush().expect("flush stdin");
    std::thread::sleep(Duration::from_millis(500));
    let kill = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("run kill");
    assert!(kill.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll slugaudit") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let Some(status) = status else {
        child.kill().expect("kill hung slugaudit");
        child.wait().expect("reap hung slugaudit");
        panic!("slugaudit did not exit within 5 s of SIGTERM with stdin open");
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr pipe")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("received SIGTERM"), "{stderr}");
    drop(stdin);
}
