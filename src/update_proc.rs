//! Bounded child-process runner for self-update helpers (`curl`, `sha256sum`,
//! target `--version`), mapping [`crate::util::run_bounded_command`] outcomes
//! onto [`UpdateError`].

use super::UpdateError;
use crate::util::{BoundedProcError, run_bounded_command};
use std::process::Command;
use std::time::Duration;

/// Largest stderr output buffered from a child process, in bytes.
const STDERR_MAX_BYTES: u64 = 64 * 1024;

/// Runs `command` with bounded stdout (when `stdout_limit` is `Some`), bounded
/// stderr (`STDERR_MAX_BYTES`), and a wall-clock `timeout`.
pub(super) fn run_bounded(
    command: &mut Command,
    tool: &'static str,
    stdout_limit: Option<u64>,
    timeout: Duration,
) -> Result<Vec<u8>, UpdateError> {
    let stdout_spec = stdout_limit.map(|limit| (limit, false));
    let output = run_bounded_command(command, tool, stdout_spec, STDERR_MAX_BYTES, timeout)
        .map_err(|error| match error {
            BoundedProcError::Spawn(source) => UpdateError::ToolSpawn { tool, source },
            BoundedProcError::Io(err) => UpdateError::Network(err.to_string()),
            BoundedProcError::TimedOut { timeout } => UpdateError::Network(format!(
                "{tool} did not finish within {}s and was stopped",
                timeout.as_secs_f32()
            )),
        })?;
    if let Some(limit) = stdout_limit
        && output.stdout_exceeded
    {
        return Err(UpdateError::Network(format!(
            "response exceeded {limit} bytes"
        )));
    }
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(UpdateError::Network(if message.is_empty() {
            format!("{tool} exited with {}", output.status)
        } else {
            message
        }));
    }
    Ok(output.stdout)
}

#[cfg(all(test, unix))]
#[path = "update_proc_tests.rs"]
mod tests;
