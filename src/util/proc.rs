//! Bounded child-process runner shared by CLI client commands (`connect` /
//! `disconnect`) and self-update helpers (`curl`, `sha256sum`, `--version`).
//!
//! Spawns the child in its own Unix process group with `stdin` closed, caps
//! captured `stdout` and `stderr` bytes on helper threads, enforces a wall-clock
//! deadline, and sends `SIGKILL` to the process group before reaping if the
//! child or an inherited pipe outlives the timeout.

use std::io::Read as _;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);

type PipeReceiver = mpsc::Receiver<std::io::Result<(Vec<u8>, bool)>>;

/// Captured result of a bounded child-process execution.
#[derive(Debug)]
pub(crate) struct BoundedOutput {
    /// Exit status of the child process.
    pub(crate) status: ExitStatus,
    /// Captured `stdout` bytes (at most `stdout_limit`).
    pub(crate) stdout: Vec<u8>,
    /// Whether `stdout` produced at least one byte beyond `stdout_limit`.
    pub(crate) stdout_exceeded: bool,
    /// Captured `stderr` bytes (at most `stderr_limit`).
    pub(crate) stderr: Vec<u8>,
}

/// Failure modes of [`run_bounded_command`].
#[derive(Debug)]
pub(crate) enum BoundedProcError {
    /// `Command::spawn` failed before a child existed.
    Spawn(std::io::Error),
    /// Waiting on or reading from the child failed.
    Io(std::io::Error),
    /// The child or its output pipes outlived `timeout` and were killed.
    TimedOut { timeout: Duration },
}

fn spawn_capped_reader(
    pipe: impl std::io::Read + Send + 'static,
    limit: u64,
    drain_tail: bool,
) -> PipeReceiver {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut pipe = pipe;
        let mut buf = Vec::new();
        // KEEP saturating: reading one byte past the cap detects overflow without wrapping.
        let cap = limit.saturating_add(1);
        let res = (&mut pipe).take(cap).read_to_end(&mut buf).and_then(|_| {
            if drain_tail {
                std::io::copy(&mut pipe, &mut std::io::sink()).map(|_| ())
            } else {
                Ok(())
            }
        });
        let outcome = res.map(|()| {
            let exceeded = match u64::try_from(buf.len()) {
                Ok(len) => len > limit,
                Err(_) => true,
            };
            if exceeded && let Ok(max_len) = usize::try_from(limit) {
                buf.truncate(max_len);
            }
            (buf, exceeded)
        });
        if tx.send(outcome).is_err() {
            tracing::debug!("child output reader finished after caller timed out");
        }
    });
    rx
}

fn collect_pipe(
    label: &str,
    pid: u32,
    rx: Option<PipeReceiver>,
    deadline: Instant,
    timeout: Duration,
) -> Result<(Vec<u8>, bool), BoundedProcError> {
    let Some(rx) = rx else {
        return Ok((Vec::new(), false));
    };
    // KEEP saturating: an elapsed deadline leaves zero wait time and triggers TimedOut immediately.
    let remaining = deadline.saturating_duration_since(Instant::now());
    let result = match rx.recv_timeout(remaining) {
        Ok(read) => read.map_err(BoundedProcError::Io),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(BoundedProcError::TimedOut { timeout }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(BoundedProcError::Io(
            std::io::Error::other(format!("{label} output reader stopped without a result")),
        )),
    };
    if result.is_err() {
        kill_process_group(label, pid);
    }
    result
}

/// Runs `command` with `stdin` closed, optional capped `stdout` (`None` sends
/// `stdout` to `Stdio::null()`), capped `stderr` (draining excess), and a
/// wall-clock `timeout`.
pub(crate) fn run_bounded_command(
    command: &mut Command,
    label: &str,
    stdout_limit: Option<(u64, bool)>,
    stderr_limit: u64,
    timeout: Duration,
) -> Result<BoundedOutput, BoundedProcError> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        BoundedProcError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the command timeout overflowed Instant",
        ))
    })?;
    let stdout_cfg = if stdout_limit.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    command
        .stdin(Stdio::null())
        .stdout(stdout_cfg)
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(BoundedProcError::Spawn)?;
    let pid = child.id();
    let stdout_rx = match (child.stdout.take(), stdout_limit) {
        (Some(pipe), Some((limit, drain_tail))) => {
            Some(spawn_capped_reader(pipe, limit, drain_tail))
        }
        _ => None,
    };
    let stderr_rx = child
        .stderr
        .take()
        .map(|pipe| spawn_capped_reader(pipe, stderr_limit, true));

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                kill_and_reap(label, &mut child);
                return Err(BoundedProcError::TimedOut { timeout });
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(err) => {
                kill_and_reap(label, &mut child);
                return Err(BoundedProcError::Io(err));
            }
        }
    };

    let (stdout, stdout_exceeded) = collect_pipe(label, pid, stdout_rx, deadline, timeout)?;
    let (stderr, _) = collect_pipe(label, pid, stderr_rx, deadline, timeout)?;
    Ok(BoundedOutput {
        status,
        stdout,
        stdout_exceeded,
        stderr,
    })
}

fn kill_and_reap(label: &str, child: &mut std::process::Child) {
    if !kill_process_group(label, child.id())
        && let Err(error) = child.kill()
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        tracing::warn!(label, error = %error, "could not kill timed-out child process");
    }
    if let Err(error) = child.wait() {
        tracing::warn!(label, error = %error, "could not reap timed-out child process");
    }
}

#[cfg(unix)]
fn kill_process_group(label: &str, pid: u32) -> bool {
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
            tracing::debug!(label, pid, %status, "kill -9 on child process group exited non-zero");
            false
        }
        Err(error) => {
            tracing::warn!(label, pid, error = %error, "could not invoke kill for child process group");
            false
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(_label: &str, _pid: u32) -> bool {
    false
}
