#![forbid(unsafe_code)]

//! Process-level exit-code contract of the `slugaudit` binary: usage errors
//! exit 2, a clean stop (stdin closed before `initialize`, SIGTERM) exits 0,
//! and no error is printed in Rust `Debug` form.

use std::io::Read;
use std::process::{Command, Output, Stdio};

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
    // Stdin stays open (held by `child.stdin`), so the server is waiting for
    // `initialize`. Give the runtime time to install its signal handler.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let kill = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("run kill");
    assert!(kill.success());
    let status = child.wait().expect("wait for slugaudit");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr pipe")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("received SIGTERM"), "{stderr}");
}
