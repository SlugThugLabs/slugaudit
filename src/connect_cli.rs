//! CLI argument grammar and process execution for the `connect` surface.
//!
//! One concern: turning a registry `Dialect` into the exact `mcp add`/`mcp
//! remove` argv a client documents, and running it. Config-file merging lives
//! in [`crate::connect_exec`] and the user-facing wording in
//! [`crate::connect_guidance`].

use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, Dialect};
use std::path::Path;
use std::process::{Command, Stdio};

/// True when the client's own `mcp list` reports slugaudit.
///
/// Advisory: a spawn failure reads as "not connected" and a non-zero exit
/// still has its stdout scanned; both are logged with the cause.
pub fn check_connected(cli: &str) -> bool {
    let output = match Command::new(cli)
        .args(["mcp", "list"])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(cli, error = %error, "could not run `mcp list`; treating the client as not connected");
            return false;
        }
    };
    list_shows_slugaudit(cli, &output)
}

/// Scans an `mcp list` result for the server; an unsuccessful exit is logged
/// and its output is still scanned.
pub(crate) fn list_shows_slugaudit(cli: &str, output: &std::process::Output) -> bool {
    if !output.status.success() {
        tracing::warn!(cli, status = %output.status, "`mcp list` exited unsuccessfully; its output is still scanned");
    }
    String::from_utf8_lossy(&output.stdout).contains("slugaudit")
}

/// The `mcp add` argv for this client's grammar, matching its own usage line.
pub fn add_args(agent: &AgentDef, binary: &Path) -> Vec<String> {
    let bin = binary.display().to_string();
    let mut args = vec!["mcp".to_string(), "add".to_string()];
    match agent.dialect {
        Dialect::Claude => args.extend(["-s", "user", "slugaudit", "--"].map(str::to_string)),
        Dialect::GeminiScope => args.extend(["-s", "user", "slugaudit"].map(str::to_string)),
        Dialect::Hermes | Dialect::Crush => {
            args.extend(["slugaudit", "--command"].map(str::to_string));
        }
        Dialect::Q => args.extend(["--name", "slugaudit", "--command"].map(str::to_string)),
        Dialect::TransportStdio => {
            args.extend(["slugaudit", "--transport", "stdio"].map(str::to_string));
        }
        Dialect::DroidType => args.extend(["--type", "stdio", "slugaudit"].map(str::to_string)),
        Dialect::BobTransportScope => args
            .extend(["--transport", "stdio", "--scope", "global", "slugaudit"].map(str::to_string)),
        Dialect::DashSeparator => {
            args.push("slugaudit".to_string());
            if agent.id == "grok" {
                args.extend(["--scope", "user"].map(str::to_string));
            }
            args.push("--".to_string());
        }
        Dialect::Plain => args.push("slugaudit".to_string()),
    }
    args.push(bin);
    args
}

/// The `mcp remove` argv, only ever run by `disconnect`: a failed remove during
/// `connect` could deregister a working server.
fn remove_args(agent: &AgentDef) -> Vec<String> {
    let mut args = vec!["mcp".to_string(), "remove".to_string()];
    match agent.dialect {
        Dialect::Claude | Dialect::GeminiScope => {
            args.extend(["-s", "user", "slugaudit"].map(str::to_string));
        }
        Dialect::Q => args.extend(["--name", "slugaudit"].map(str::to_string)),
        Dialect::DashSeparator if agent.id == "grok" => {
            args.extend(["slugaudit", "--scope", "user"].map(str::to_string));
        }
        // Bob's scope defaults to *workspace*, so the global entry we added
        // would survive a bare `mcp remove slugaudit`.
        Dialect::BobTransportScope => {
            args.extend(["--scope", "global", "slugaudit"].map(str::to_string));
        }
        _ => args.push("slugaudit".to_string()),
    }
    args
}

/// The command line as the user would type it, for the "Tried:"/"Ran:" lines.
pub fn display_command(cli: &str, args: &[String]) -> String {
    format!("{cli} {}", args.join(" "))
}

/// Runs the client's own `mcp add` command line for one agent registration.
///
/// # Errors
///
/// Returns the client's recorded failure text as a `ConnectError`.
pub fn run_add(cli: &str, args: &[String]) -> Result<(), ConnectError> {
    let output = Command::new(cli)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|inner| ConnectError::AgentCommand {
            cli: cli.to_string(),
            inner,
        })?;
    if !output.status.success() {
        let status = output.status.to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.is_empty() {
            // `AddFailed` tells the user to look above; with no captured
            // output that instruction would be false.
            return Err(ConnectError::AddFailedNoOutput {
                cli: cli.to_string(),
                status,
            });
        }
        eprintln!("{stderr}");
        return Err(ConnectError::AddFailed {
            cli: cli.to_string(),
            status,
        });
    }
    Ok(())
}

/// Runs the client's own `mcp remove` command for a best-effort disconnect.
///
/// # Errors
///
/// Returns the client's recorded failure text; the file fallback may still succeed.
pub fn run_remove(agent: &AgentDef, cli: &str) -> Result<(), ConnectError> {
    let status = Command::new(cli)
        .args(remove_args(agent))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|inner| ConnectError::AgentCommand {
            cli: cli.to_string(),
            inner,
        })?;
    if !status.success() {
        // `RemoveFailed`, not `AddFailed`: the latter names the wrong command
        // and tells the user to "see the output above" while this call nulls
        // both streams, so there is no output above to see.
        return Err(ConnectError::RemoveFailed {
            cli: cli.to_string(),
            status: status.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cli_that_cannot_be_spawned_is_not_connected_and_says_why() {
        let (connected, logs) = crate::store::test_capture::capture_warns(|| {
            check_connected("/nonexistent/slugaudit-fake-cli")
        });

        assert!(!connected);
        assert!(logs.contains("could not run `mcp list`"), "{logs}");
    }

    #[cfg(unix)]
    #[test]
    fn a_list_that_shows_slugaudit_but_exits_nonzero_is_connected_and_logged() {
        use std::os::unix::process::ExitStatusExt as _;
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(3 << 8),
            stdout: b"slugaudit\n".to_vec(),
            stderr: Vec::new(),
        };

        let (connected, logs) =
            crate::store::test_capture::capture_warns(|| list_shows_slugaudit("fake-cli", &output));

        assert!(connected);
        assert!(logs.contains("exited unsuccessfully"), "{logs}");
    }

    #[test]
    fn an_add_that_cannot_be_spawned_is_reported_as_an_agent_command_failure() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
        let empty = tempfile::tempdir().expect("empty bin dir");
        let args = ["mcp", "add", "slugaudit", "--", "/bin/slugaudit"].map(str::to_string);

        let result = temp_env::with_var("PATH", Some(empty.path().as_os_str()), || {
            run_add("claude", &args)
        });

        let Err(ConnectError::AgentCommand { cli, inner }) = result else {
            panic!("expected AgentCommand, got {result:?}");
        };
        assert_eq!(cli, "claude");
        assert_eq!(
            inner.kind(),
            std::io::ErrorKind::NotFound,
            "the missing executable must be the cause: {inner}"
        );
    }
}
