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

/// Takes the captured stream, waiting no later than `deadline`. A reader that
/// is still blocked then is holding a pipe the child handed to a background
/// process, so the stream is abandoned and the run times out.
fn collect(
    cli: &str,
    receiver: &Captured,
    deadline: Instant,
    timeout: Duration,
) -> Result<Vec<u8>, ConnectError> {
    let command_error = |inner| ConnectError::AgentCommand {
        cli: cli.to_string(),
        inner,
    };
    // KEEP saturating: a deadline already passed has no time left, and a zero wait is exactly the Timeout answer
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(read) => read.map_err(command_error),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ConnectError::AgentTimeout {
            cli: cli.to_string(),
            timeout_secs: timeout.as_secs_f32(),
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(command_error(std::io::Error::other(
            "the client output reader stopped without a result",
        ))),
    }
}

/// Runs `cli args...` with stdin closed and both output streams captured,
/// waiting at most `timeout`.
///
/// # Errors
///
/// `AgentCommand` when the client cannot be spawned, waited on or read;
/// `AgentTimeout` when it outlives `timeout` (the child is killed and reaped
/// first) or when its output streams stay open past `timeout` after it
/// exited, as when it left a background process holding them. The error is
/// returned, not logged, so the caller reports it once.
pub fn run_bounded(cli: &str, args: &[String], timeout: Duration) -> Result<Output, ConnectError> {
    let command_error = |inner| ConnectError::AgentCommand {
        cli: cli.to_string(),
        inner,
    };
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        command_error(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the timeout is too large to schedule a deadline",
        ))
    })?;
    let mut command = Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(command_error)?;
    let pid = child.id();
    let stdout = child.stdout.take().map(capture);
    let stderr = child.stderr.take().map(capture);
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
        Some(receiver) => collect(cli, &receiver, deadline, timeout).inspect_err(|_| {
            kill_process_group(cli, pid);
        }),
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
    kill_and_reap(cli, child);
    ConnectError::AgentTimeout {
        cli: cli.to_string(),
        timeout_secs: timeout.as_secs_f32(),
    }
}

/// Kills the child (and on Unix its entire process group, so grandchildren
/// holding stdout/stderr pipes cannot outlive the timeout) and reaps it. A
/// failure here is logged; the caller is already returning an error.
fn kill_and_reap(cli: &str, child: &mut std::process::Child) {
    if !kill_process_group(cli, child.id())
        && let Err(error) = child.kill()
    {
        tracing::warn!(cli, error = %error, "could not kill the client CLI");
    }
    if let Err(error) = child.wait() {
        tracing::warn!(cli, error = %error, "could not reap the client CLI");
    }
}

/// Sends `SIGKILL` to the Unix process group `pid` (created via
/// `CommandExt::process_group(0)`), returning `true` when `kill` succeeded.
#[cfg(unix)]
fn kill_process_group(cli: &str, pid: u32) -> bool {
    let target = format!("-{pid}");
    let run = |program: &str| {
        Command::new(program)
            .args(["-9", &target])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };
    let result = match run("kill") {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => run("/bin/kill"),
        other => other,
    };
    match result {
        Ok(status) if status.success() => true,
        Ok(status) => {
            tracing::debug!(
                cli,
                pid,
                %status,
                "kill -9 on client process group exited non-zero"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                cli,
                pid,
                error = %error,
                "could not invoke kill for client process group"
            );
            false
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(_cli: &str, _pid: u32) -> bool {
    false
}

#[cfg(all(test, unix))]
#[path = "connect_run_tests.rs"]
mod tests;
