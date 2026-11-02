#![forbid(unsafe_code)]

//! Process-level exit-code contract of the `slugaudit` binary: usage errors
//! exit 2, a clean stop (stdin closed before `initialize`, SIGTERM) exits 0,
//! and no error is printed in Rust `Debug` form.

use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn slugaudit(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_slugaudit"));
    command.args(args);
    command
}

fn run(args: &[&str]) -> TestResult<Output> {
    slugaudit(args)
        .stdin(Stdio::null())
        .output()
        .check("run slugaudit")
}

#[test]
fn cli_exit_unknown_flag_exits_2_with_a_display_message() -> TestResult {
    let output = run(&["--bogus"])?;
    let stderr = String::from_utf8(output.stderr).check("utf-8 stderr")?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.starts_with("Error: unknown command \"--bogus\""),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn cli_exit_lowercase_v_prints_the_version_and_exits_0() -> TestResult {
    let output = run(&["-v"])?;
    let stdout = String::from_utf8(output.stdout).check("utf-8 stdout")?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout, format!("slugaudit {}\n", env!("CARGO_PKG_VERSION")));
    Ok(())
}

#[test]
fn cli_exit_connect_with_an_extra_argument_exits_2() -> TestResult {
    let output = run(&["connect", "claude", "extra"])?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).check("utf-8 stderr")?;
    assert!(stderr.contains("unexpected argument \"extra\""), "{stderr}");
    Ok(())
}

#[test]
fn cli_exit_stdin_closed_before_initialize_exits_0_with_one_clean_line() -> TestResult {
    let output = run(&[])?;
    let stderr = String::from_utf8(output.stderr).check("utf-8 stderr")?;
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains("ConnectionClosed("), "{stderr}");
    Ok(())
}

#[test]
fn cli_exit_sigterm_to_a_serving_process_exits_0() -> TestResult {
    let mut child = slugaudit(&[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .check("spawn slugaudit")?;
    // Like a real host: complete the handshake and keep stdin open until
    // after the exit assertion, so the stdin reader thread stays parked.
    let mut stdin = child.stdin.take().check("stdin pipe")?;
    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    writeln!(stdin, "{initialize}").check("write initialize")?;
    let mut stdout = BufReader::new(child.stdout.take().check("stdout pipe")?);
    let mut reply = String::new();
    stdout
        .read_line(&mut reply)
        .check("read initialize reply")?;
    assert!(reply.contains("\"result\""), "{reply}");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .check("write initialized")?;
    stdin.flush().check("flush stdin")?;
    std::thread::sleep(Duration::from_millis(500));
    let kill = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .check("run kill")?;
    assert!(kill.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().check("poll slugaudit")? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let Some(status) = status else {
        child.kill().check("kill hung slugaudit")?;
        child.wait().check("reap hung slugaudit")?;
        panic!("slugaudit did not exit within 5 s of SIGTERM with stdin open");
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .check("stderr pipe")?
        .read_to_string(&mut stderr)
        .check("read stderr")?;
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("received SIGTERM"), "{stderr}");
    drop(stdin);
    Ok(())
}

#[test]
fn every_subcommand_help_prints_its_usage_and_exits_0_without_running_it() -> TestResult {
    let home = tempfile::tempdir().check("temp home")?;
    let empty_path = tempfile::tempdir().check("empty PATH dir")?;
    for name in [
        "serve",
        "connect",
        "disconnect",
        "remove",
        "install",
        "menu",
        "update",
    ] {
        for args in [
            vec![name, "--help"],
            vec![name, "x", "--help"],
            vec![name, "-h"],
        ] {
            let output = slugaudit(&args)
                .env("HOME", home.path())
                .env("SLUGTHUG_HOME", home.path())
                .env("SLUGTHUG_CONFIG_HOME", home.path())
                .env("PATH", empty_path.path())
                .stdin(Stdio::null())
                .output()
                .check("run slugaudit")?;
            let stdout = String::from_utf8(output.stdout).check("utf-8 stdout")?;
            assert_eq!(output.status.code(), Some(0), "{args:?}");
            assert!(
                stdout.starts_with("USAGE:\n    slugaudit "),
                "{args:?}: {stdout}"
            );
            assert!(output.stderr.is_empty(), "{args:?}");
        }
    }
    let leftovers = std::fs::read_dir(home.path()).check("read home")?.count();
    assert_eq!(leftovers, 0, "help must not touch the home directory");
    let config_dir = std::fs::read_dir(home.path().join(".slugthug"))
        .check_err("help must not create the config directory")?;
    assert_eq!(config_dir.kind(), std::io::ErrorKind::NotFound);
    Ok(())
}
