//! Registration (`connect`) and deregistration (`disconnect`) policy:
//! verified CLI dispatch (`mcp add` / `mcp remove`) and JSON/JSONC config
//! fallback for files SlugAudit writes.

use super::ConnectError;
use super::cli as connect_cli;
use super::config_file::{remove_json_slugaudit, write_agent_config};
use super::guidance;
use super::registry_types::{AgentDef, Strategy};
use std::path::Path;

/// What a disconnect actually did, so the caller does not report success for
/// a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectOutcome {
    /// At least one registration route was touched.
    Disconnected,
    /// There was no config entry and no installed client CLI to remove.
    NothingToDo,
}

fn installed_cli(agent: &AgentDef) -> Option<&'static str> {
    agent.cli.filter(|cli| crate::probe::cli_installed(cli))
}

fn can_write_fallback(agent: &AgentDef) -> bool {
    agent.file_fallback && guidance::config_writable(agent)
}

fn prepare_add(
    agent: &AgentDef,
    binary: &Path,
    cli: &str,
) -> Result<(Vec<String>, String), ConnectError> {
    let args = connect_cli::add_args(agent, binary)?;
    let attempted = connect_cli::display_command(cli, &args);
    Ok((args, attempted))
}

/// Registers SlugAudit with one agent: its CLI when verified to have one, its
/// config file when the written shape is established, otherwise an actionable
/// refusal — never a guessed write.
///
/// # Errors
///
/// Every refusal names what was tried and the manual step that works.
pub fn connect_agent(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
) -> Result<(), ConnectError> {
    if agent.blocker().is_some() {
        return Err(guidance::refusal(agent, binary, home));
    }
    match agent.strategy {
        Strategy::ConfigFile | Strategy::CliInteractiveOnly => {
            write_and_report(agent, binary, home, None)
        }
        Strategy::CliDirect => connect_via_cli(agent, binary, home),
        Strategy::Unknown => Err(guidance::refusal(agent, binary, home)),
    }
}

/// Writes the client config and, when the write fails, prints the failure
/// guidance before returning the error.
fn write_and_report(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    note: Option<&str>,
) -> Result<(), ConnectError> {
    write_agent_config(agent, binary, home, note).map_err(|err| {
        let action = format!("write {}", guidance::target(agent, home));
        report_failure(agent, binary, &action, None, None, err)
    })
}

fn connect_via_cli(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
) -> Result<(), ConnectError> {
    if let Some(cli) = installed_cli(agent) {
        return run_installed_cli(agent, binary, home, cli);
    }
    let Some(cli) = agent.cli else {
        return Err(guidance::refusal(agent, binary, home));
    };
    if can_write_fallback(agent) {
        let note = guidance::fallback_note(agent, home, cli, "was not found on PATH");
        return write_and_report(agent, binary, home, Some(&note));
    }
    let (_, display) = prepare_add(agent, binary, cli)?;
    Err(guidance::cli_absent(agent, binary, cli, &display))
}

fn run_installed_cli(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    cli: &str,
) -> Result<(), ConnectError> {
    let (args, attempted) = prepare_add(agent, binary, cli)?;
    match connect_cli::run_add(cli, &args) {
        Ok(()) => guidance::print_lines(&guidance::success_lines(
            agent,
            &format!("Ran: {attempted}"),
        )),
        Err(cli_err) if can_write_fallback(agent) => {
            let note = guidance::fallback_note(
                agent,
                home,
                cli,
                &format!("rejected `mcp add` ({cli_err})"),
            );
            write_agent_config(agent, binary, home, Some(&note)).map_err(|file_err| {
                let action = format!("write {}", guidance::target(agent, home));
                let why = format!("{cli_err}; the file fallback failed too: {file_err}");
                report_failure(agent, binary, &action, Some(&attempted), Some(why), cli_err)
            })
        }
        Err(cli_err) => Err(report_failure(
            agent,
            binary,
            &attempted,
            Some(&attempted),
            None,
            cli_err,
        )),
    }
}

fn report_failure(
    agent: &AgentDef,
    binary: &Path,
    attempted: &str,
    hint: Option<&str>,
    detail: Option<String>,
    source: ConnectError,
) -> ConnectError {
    let why = match detail {
        Some(detail) => detail,
        None => source.to_string(),
    };
    guidance::keep_source_error(
        source,
        guidance::print_lines(&guidance::failure_lines(
            agent, binary, attempted, hint, &why,
        )),
    )
}

/// Result of attempting the CLI `mcp remove` route during [`disconnect_agent`].
enum CliDisconnect {
    /// Agent is not CLI-driven or its verified CLI executable is not installed.
    NotApplicable,
    /// Client CLI `mcp remove` exited successfully.
    Removed,
    /// Client CLI failed; caller decides whether a JSON fallback rescues it.
    Failed {
        cli: &'static str,
        error: ConnectError,
    },
}

fn try_cli_disconnect(agent: &AgentDef) -> CliDisconnect {
    if !matches!(agent.strategy, Strategy::CliDirect) {
        return CliDisconnect::NotApplicable;
    }
    let Some(cli) = installed_cli(agent) else {
        return CliDisconnect::NotApplicable;
    };
    match connect_cli::run_remove(agent, cli) {
        Ok(()) => CliDisconnect::Removed,
        Err(error) => CliDisconnect::Failed { cli, error },
    }
}

fn try_json_disconnect(agent: &AgentDef, home: Option<&Path>) -> Result<bool, ConnectError> {
    if !agent.writes_config_file() || agent.shape.write_section().is_none() {
        return Ok(false);
    }
    let Some(path) = home.and_then(|dir| agent.config.resolve(dir)) else {
        return Ok(false);
    };
    remove_json_slugaudit(&path, agent.shape.sections())
}

/// Removes SlugAudit from one agent: the CLI route when its verified binary is
/// present, and the file entry only for configs this tool writes.
///
/// # Errors
///
/// Fails when the config file entry cannot be read or rewritten, when the
/// client's remove command fails and no fallback JSON entry rescued it, or when
/// a CLI-only client's config still contains `slugaudit` while its CLI is
/// absent from `PATH`.
pub fn disconnect_agent(
    agent: &AgentDef,
    home: Option<&Path>,
) -> Result<DisconnectOutcome, ConnectError> {
    let allow_fallback =
        home.is_some() && agent.file_fallback && agent.shape.write_section().is_some();
    let cli_outcome = try_cli_disconnect(agent);
    if !allow_fallback && let CliDisconnect::Failed { error, .. } = cli_outcome {
        return Err(error);
    }
    let json_removed = try_json_disconnect(agent, home)?;
    match (cli_outcome, json_removed) {
        (CliDisconnect::Failed { cli, error }, true) => {
            tracing::warn!(
                agent = agent.id,
                cli,
                error = %error,
                "CLI remove failed, but fallback JSON entry was removed",
            );
            Ok(DisconnectOutcome::Disconnected)
        }
        (CliDisconnect::Failed { error, .. }, false) => Err(error),
        (CliDisconnect::Removed, _) | (CliDisconnect::NotApplicable, true) => {
            Ok(DisconnectOutcome::Disconnected)
        }
        (CliDisconnect::NotApplicable, false) => {
            if agent.strategy == Strategy::CliDirect
                && let Some(cli) = agent.cli
                && agent.is_connected(home)?
            {
                return Err(ConnectError::DisconnectCliMissing {
                    agent: agent.display_name.to_owned(),
                    cli: cli.to_owned(),
                    path: guidance::target(agent, home),
                });
            }
            Ok(DisconnectOutcome::NothingToDo)
        }
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod tests;
