//! Bounded execution of a client CLI.
//!
//! Every client command `connect`/`disconnect` runs goes through here, so a
//! client that hangs (a login prompt, a network wait) cannot hang SlugAudit:
//! past the deadline the child is killed and reaped and the caller gets a
//! [`ConnectError::AgentTimeout`].

use super::ConnectError;
use crate::util::{BoundedProcError, run_bounded_command};
use std::process::{Command, Output};
use std::time::Duration;

/// How long one client CLI invocation may run before it is killed.
pub(super) const CLIENT_CLI_TIMEOUT: Duration = Duration::from_secs(15);

/// Most bytes kept from each of the child's output streams; the rest is drained
/// and discarded so a chatty client cannot fill memory or block on a full pipe.
const MAX_CAPTURED_BYTES: u64 = 1024 * 1024;

/// Runs `cli args...` with stdin closed and both output streams captured,
/// waiting at most `timeout`.
///
/// # Errors
///
/// `AgentCommand` when the client cannot be spawned, waited on or read;
/// `AgentTimeout` when it outlives `timeout` (the child is killed and reaped
/// first) or when its output streams stay open past `timeout` after it
/// exited, as when it left a background process holding them.
pub(crate) fn run_bounded(
    cli: &str,
    args: &[String],
    timeout: Duration,
) -> Result<Output, ConnectError> {
    let mut command = Command::new(cli);
    command.args(args);
    match run_bounded_command(
        &mut command,
        cli,
        Some((MAX_CAPTURED_BYTES, true)),
        MAX_CAPTURED_BYTES,
        timeout,
    ) {
        Ok(output) => Ok(Output {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }),
        Err(BoundedProcError::Spawn(inner) | BoundedProcError::Io(inner)) => {
            Err(ConnectError::AgentCommand {
                cli: cli.to_owned(),
                inner,
            })
        }
        Err(BoundedProcError::TimedOut { timeout }) => Err(ConnectError::AgentTimeout {
            cli: cli.to_owned(),
            timeout_secs: timeout.as_secs_f32(),
        }),
    }
}

#[cfg(all(test, unix))]
#[path = "run_tests.rs"]
mod tests;
