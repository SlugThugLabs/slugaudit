//! Registering or removing this binary as the `slugaudit` MCP server in an AI agent or editor.

pub(crate) mod config_atomic;
pub(crate) mod config_file;
mod interactive;
pub(crate) mod registry_types;

#[cfg(test)]
use interactive::{answer_from, parse_choice, write_prompt};
pub use interactive::{run_connect_interactive, run_disconnect_interactive};

use crate::cli;
use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, find_agent};
use crate::connect_exec::{DisconnectOutcome, connect_agent, disconnect_agent};
use crate::install::{config_home, running_binary, slugaudit_dir};
use std::path::{Path, PathBuf};

/// Registers SlugAudit as an MCP server for the named agent, or prompts interactively.
///
/// # Errors
///
/// Fails when `agent_name` is not a registered agent, when registration fails
/// (the refusal, config write, or client CLI error is returned as is), or, in
/// the interactive path, when the prompt cannot be read or the answer is not a
/// valid choice.
pub fn run_connect(agent_name: Option<&str>) -> Result<(), ConnectError> {
    match agent_name {
        Some(name) => {
            let agent =
                find_agent(name).ok_or_else(|| ConnectError::UnknownAgent(name.to_string()))?;
            let connected = connection_state(&agent, config_home().as_deref())?;
            connect_one(agent, connected)
        }
        None => run_connect_interactive(),
    }
}

/// Disconnects SlugAudit from the named agent, or prompts interactively.
///
/// # Errors
///
/// Fails when `agent_name` is not a registered agent, when removing the
/// registration fails, or, in the interactive path, when the prompt cannot be
/// read or the answer is not a valid choice.
pub fn run_disconnect(agent_name: Option<&str>) -> Result<(), ConnectError> {
    match agent_name {
        Some(name) => {
            let agent =
                find_agent(name).ok_or_else(|| ConnectError::UnknownAgent(name.to_string()))?;
            disconnect_one(agent)
        }
        None => run_disconnect_interactive(),
    }
}

/// The stable installed binary when one exists, else `current`.
///
/// # Errors
///
/// A stat failure on the installed path is returned rather than read as
/// "not installed", which would register a possibly temporary `current`.
pub(crate) fn prefer_slugthug_binary(current: &Path) -> Result<PathBuf, std::io::Error> {
    if let Some(slugaudit) = slugaudit_dir() {
        let candidate = slugaudit.join("slugaudit");
        if candidate.try_exists()? {
            return Ok(candidate);
        }
    }
    Ok(current.to_path_buf())
}

/// Maps the running-binary lookup onto the connect error, keeping the I/O
/// cause. Split from [`connect_one`] so the failure arm is testable without
/// faking the process that owns the executable path.
fn binary_path(result: Result<PathBuf, std::io::Error>) -> Result<PathBuf, ConnectError> {
    result.map_err(ConnectError::BinaryPath)
}

/// Whether `agent` already has SlugAudit registered. A config that cannot be
/// examined is reported on stderr as a note and counts as not connected: the
/// answer only picks the wording of the next prompt, and the registration
/// itself fails with its own error if the config is unusable.
fn connection_state(agent: &AgentDef, home: Option<&Path>) -> Result<bool, ConnectError> {
    match agent.is_connected(home) {
        Ok(connected) => Ok(connected),
        Err(failure) => {
            cli::err(format_args!("Note: {failure}\n"))?;
            Ok(false)
        }
    }
}

/// Registers with `agent`; `already_connected` only chooses the progress line.
fn connect_one(agent: AgentDef, already_connected: bool) -> Result<(), ConnectError> {
    let binary = binary_path(running_binary())?;
    let binary = prefer_slugthug_binary(&binary)?;
    let home = config_home();
    if already_connected {
        cli::out(format_args!(
            "SlugAudit is already registered with {}. Updating connection...\n",
            agent.display_name
        ))?;
    } else {
        cli::out(format_args!(
            "Connecting SlugAudit to {}...\n",
            agent.display_name
        ))?;
    }
    connect_agent(&agent, &binary, home.as_deref())?;
    cli::out(format_args!("Done.\n"))?;
    if let Some(client) = agent.cli {
        cli::out(format_args!("Verify with: {client} mcp list\n"))?;
    }
    Ok(())
}

fn disconnect_one(agent: AgentDef) -> Result<(), ConnectError> {
    let home = config_home();
    cli::out(format_args!(
        "Disconnecting SlugAudit from {}...\n",
        agent.display_name
    ))?;
    match disconnect_agent(&agent, home.as_deref())? {
        DisconnectOutcome::Disconnected => cli::out(format_args!("Done.\n"))?,
        DisconnectOutcome::NothingToDo => cli::out(format_args!("Nothing to disconnect.\n"))?,
    }
    Ok(())
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
