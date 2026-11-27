//! CLI argument grammar and process execution for the `connect` surface.
//!
//! One concern: turning a registry [`Dialect`] into the exact `mcp add`/`mcp
//! remove` argv a client documents, and running it. Config-file merging lives
//! in `exec` and the user-facing wording in `guidance`.

use super::ConnectError;
use super::guidance;
use super::registry_types::{AgentDef, Dialect, binary_text};
use super::run::{CLIENT_CLI_TIMEOUT, run_bounded};
use std::path::Path;
use std::time::Duration;

/// The `mcp add` argv for this client's grammar, matching its own usage line.
///
/// # Errors
///
/// A binary path that is not valid UTF-8 is refused: no client can represent
/// it as a command, so no argv is built.
pub(super) fn add_args(agent: &AgentDef, binary: &Path) -> Result<Vec<String>, ConnectError> {
    let bin = binary_text(binary)?;
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
            args.extend(["slugaudit", "--"].map(str::to_string));
        }
        Dialect::DashSeparatorUserScope => {
            args.extend(["slugaudit", "--scope", "user", "--"].map(str::to_string));
        }
        Dialect::Plain => args.push("slugaudit".to_string()),
    }
    args.push(bin.to_owned());
    Ok(args)
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
        Dialect::DashSeparatorUserScope => {
            args.extend(["slugaudit", "--scope", "user"].map(str::to_string));
        }
        // Bob's scope defaults to *workspace*, so the global entry we added
        // would survive a bare `mcp remove slugaudit`.
        Dialect::BobTransportScope => {
            args.extend(["--scope", "global", "slugaudit"].map(str::to_string));
        }
        Dialect::Plain
        | Dialect::Hermes
        | Dialect::DashSeparator
        | Dialect::Crush
        | Dialect::TransportStdio
        | Dialect::DroidType => args.push("slugaudit".to_string()),
    }
    args
}

/// The command line as the user would type it, for the "Tried:"/"Ran:" lines.
/// Every argument is shell-quoted, so a copied line cannot split or expand.
#[must_use]
pub(super) fn display_command(cli: &str, args: &[String]) -> String {
    let quoted: Vec<String> = args.iter().map(|arg| shell_quote(arg)).collect();
    format!("{cli} {}", quoted.join(" "))
}

/// Quotes one argv element for display: safe characters stay bare, while an
/// empty value or anything a shell could split or expand is single-quoted,
/// with an embedded `'` rendered as `'\''`.
fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:=@%+,/".contains(&byte))
    {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Runs the client's own `mcp add` command line for one agent registration.
///
/// # Errors
///
/// Returns the client's recorded failure text as a `ConnectError`.
pub(super) fn run_add(cli: &str, args: &[String]) -> Result<(), ConnectError> {
    run_add_within(cli, args, CLIENT_CLI_TIMEOUT)
}

/// [`run_add`] with an explicit deadline.
fn run_add_within(cli: &str, args: &[String], timeout: Duration) -> Result<(), ConnectError> {
    let output = run_bounded(cli, args, timeout)?;
    if !output.status.success() {
        let status = output.status.to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let captured = if stderr.is_empty() { &stdout } else { &stderr };
        if captured.is_empty() {
            // `AddFailed` tells the user to look above; with no captured
            // output that instruction would be false.
            return Err(ConnectError::AddFailedNoOutput {
                cli: cli.to_string(),
                status,
            });
        }
        let printed = crate::cli::err(format_args!("{captured}\n")).map_err(ConnectError::Io);
        return Err(guidance::keep_source_error(
            ConnectError::AddFailed {
                cli: cli.to_string(),
                status,
            },
            printed,
        ));
    }
    Ok(())
}

/// Runs the client's own `mcp remove` command for a best-effort disconnect.
///
/// # Errors
///
/// Returns the client's recorded failure text; the file fallback may still succeed.
pub(super) fn run_remove(agent: &AgentDef, cli: &str) -> Result<(), ConnectError> {
    let status = run_bounded(cli, &remove_args(agent), CLIENT_CLI_TIMEOUT)?.status;
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
#[path = "cli_tests.rs"]
mod tests;
