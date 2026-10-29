//! Bounded execution of a client CLI.
//!
//! Every client command `connect`/`disconnect` runs goes through here, so a
//! client that hangs (a login prompt, a network wait) cannot hang SlugAudit:
//! past the deadline the child is killed and reaped and the caller gets a
//! [`ConnectError::AgentTimeout`].

use crate::cli::ConnectError;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long one client CLI invocation may run before it is killed.
pub const CLIENT_CLI_TIMEOUT: Duration = Duration::from_secs(15);

/// Interval between `try_wait` polls of the child.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Most bytes kept from each of the child's output streams; the rest is drained
/// and discarded so a chatty client cannot fill memory or block on a full pipe.
const MAX_CAPTURED_BYTES: u64 = 1024 * 1024;

type Captured = mpsc::Receiver<std::io::Result<Vec<u8>>>;

/// Reads at most [`MAX_CAPTURED_BYTES`] from `stream` on a helper thread, then
/// drains the remainder so the child never blocks on a full pipe.
fn capture(stream: impl Read + Send + 'static) -> Captured {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut stream = stream;
        let mut kept = Vec::new();
        let read = (&mut stream)
            .take(MAX_CAPTURED_BYTES)
            .read_to_end(&mut kept)
            .and_then(|_| std::io::copy(&mut stream, &mut std::io::sink()).map(|_| ()));
        // The receiver is gone only when the caller timed out and moved on.
        if sender.send(read.map(|()| kept)).is_err() {
            tracing::debug!("client output reader finished after its caller gave up");
        }
    });
    receiver
}

fn collect(cli: &str, receiver: &Captured) -> Result<Vec<u8>, ConnectError> {
    receiver
        .recv()
        .map_err(|error| ConnectError::AgentCommand {
            cli: cli.to_string(),
            inner: std::io::Error::other(error),
        })?
        .map_err(|inner| ConnectError::AgentCommand {
            cli: cli.to_string(),
            inner,
        })
}

/// Runs `cli args...` with stdin closed and both output streams captured,
/// waiting at most `timeout`.
///
/// # Errors
///
/// `AgentCommand` when the client cannot be spawned, waited on or read;
/// `AgentTimeout` when it outlives `timeout` (the child is killed and reaped
/// first, and the timeout is logged once).
pub fn run_bounded(cli: &str, args: &[String], timeout: Duration) -> Result<Output, ConnectError> {
    let command_error = |inner| ConnectError::AgentCommand {
        cli: cli.to_string(),
        inner,
    };
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(command_error)?;
    let stdout = child.stdout.take().map(capture);
    let stderr = child.stderr.take().map(capture);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                return Err(stop_timed_out(cli, &mut child, timeout));
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(inner) => {
                kill_and_reap(cli, &mut child);
                return Err(command_error(inner));
            }
        }
    };
    let read = |receiver: Option<Captured>| match receiver {
        Some(receiver) => collect(cli, &receiver),
        None => Ok(Vec::new()),
    };
    Ok(Output {
        status,
        stdout: read(stdout)?,
        stderr: read(stderr)?,
    })
}

/// Kills and reaps a child that outlived its deadline and builds the error.
fn stop_timed_out(cli: &str, child: &mut std::process::Child, timeout: Duration) -> ConnectError {
    tracing::error!(
        cli,
        timeout_secs = timeout.as_secs_f32(),
        "client CLI timed out and was killed"
    );
    kill_and_reap(cli, child);
    ConnectError::AgentTimeout {
        cli: cli.to_string(),
        timeout_secs: timeout.as_secs_f32(),
    }
}

/// Kills the child and waits for it so it does not linger as a zombie. A
/// failure here is logged; the caller is already returning an error.
fn kill_and_reap(cli: &str, child: &mut std::process::Child) {
    if let Err(error) = child.kill() {
        tracing::warn!(cli, error = %error, "could not kill the client CLI");
    }
    if let Err(error) = child.wait() {
        tracing::warn!(cli, error = %error, "could not reap the client CLI");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    /// Writes an executable script. Callers hold `TEST_ENV_LOCK` so no other
    /// test has swapped `PATH` away while the script runs.
    fn script(dir: &std::path::Path, body: &str) -> String {
        let path = dir.join("fake-client");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path.display().to_string()
    }

    #[test]
    fn a_blocking_client_times_out_and_the_child_is_killed_and_reaped() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("dir");
        let pid_file = dir.path().join("pid");
        let cli = script(
            dir.path(),
            &format!("echo $$ > '{}'\nexec /bin/sleep 30", pid_file.display()),
        );

        let started = Instant::now();
        let result = run_bounded(&cli, &[], Duration::from_millis(400));

        let Err(ConnectError::AgentTimeout {
            cli: named,
            timeout_secs,
        }) = result
        else {
            panic!("expected AgentTimeout, got {result:?}");
        };
        assert_eq!(named, cli);
        assert!((timeout_secs - 0.4).abs() < 0.01, "{timeout_secs}");
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid = std::fs::read_to_string(&pid_file).expect("pid file");
        // A reaped child has no /proc entry; a killed-but-unreaped one would
        // linger as a zombie.
        assert!(
            !std::path::Path::new("/proc").join(pid.trim()).exists(),
            "child {} must be gone",
            pid.trim()
        );
    }

    #[test]
    fn a_client_that_finishes_returns_its_status_and_output() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("dir");
        let cli = script(dir.path(), "echo out; echo err >&2; exit 3");

        let output = run_bounded(&cli, &[], CLIENT_CLI_TIMEOUT).expect("runs");

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
    }

    #[test]
    fn output_beyond_the_cap_is_drained_without_blocking_the_client() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("dir");
        let cli = script(dir.path(), "/usr/bin/head -c 3000000 /dev/zero");

        let output = run_bounded(&cli, &[], CLIENT_CLI_TIMEOUT).expect("runs");

        assert!(output.status.success());
        assert_eq!(output.stdout.len() as u64, MAX_CAPTURED_BYTES);
    }
}
