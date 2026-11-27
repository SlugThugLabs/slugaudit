//! Registering or removing this binary as the `slugaudit` MCP server in an AI agent or editor.

mod agent_probe;
mod agents;
mod cli;
mod config_atomic;
mod config_file;
mod error;
mod exec;
mod guidance;
mod interactive;
mod jsonc;
mod registry_types;
pub(crate) mod run;

pub use agent_probe::{Detection, ProbeFailure, detected_agents};
pub use agents::{AGENTS, find_agent};
pub use config_file::has_slugaudit;
pub use error::ConnectError;
pub use exec::{DisconnectOutcome, connect_agent, disconnect_agent};
pub(crate) use guidance::other_agent_instructions;
pub use interactive::{run_connect_interactive, run_disconnect_interactive};
pub use registry_types::{AgentDef, ConfigPath, ConfigShape, Dialect, Evidence, Strategy};

use crate::install::{config_home, running_binary, slugaudit_dir};
use std::path::{Path, PathBuf};

/// Looks up `name` in the agent registry or returns [`ConnectError::UnknownAgent`].
fn require_agent(name: &str) -> Result<AgentDef, ConnectError> {
    find_agent(name).ok_or_else(|| ConnectError::UnknownAgent(name.to_string()))
}

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
            let agent = require_agent(name)?;
            let home = config_home();
            let connected = connection_state(&agent, home.as_deref())?;
            connect_one(agent, home.as_deref(), connected)
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
        Some(name) => disconnect_one(require_agent(name)?),
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
            crate::cli::err(format_args!("Note: {failure}\n"))?;
            Ok(false)
        }
    }
}

/// The CLI executable to suggest in `Verify with: <client> mcp list`, if any.
fn verify_client(agent: &AgentDef) -> Option<&'static str> {
    agent.verify_cli.or_else(|| {
        matches!(agent.strategy, Strategy::CliDirect)
            .then_some(agent.cli)
            .flatten()
    })
}

/// Registers with `agent`; `already_connected` only chooses the progress line.
pub(super) fn connect_one(
    agent: AgentDef,
    home: Option<&Path>,
    already_connected: bool,
) -> Result<(), ConnectError> {
    let binary = prefer_slugthug_binary(&binary_path(running_binary())?)?;
    if already_connected {
        crate::cli::out(format_args!(
            "SlugAudit is already registered with {}. Updating connection...\n",
            agent.display_name
        ))?;
    } else {
        crate::cli::out(format_args!(
            "Connecting SlugAudit to {}...\n",
            agent.display_name
        ))?;
    }
    connect_agent(&agent, &binary, home)?;
    crate::cli::out(format_args!("Done.\n"))?;
    if let Some(client) = verify_client(&agent) {
        crate::cli::out(format_args!("Verify with: {client} mcp list\n"))?;
    }
    Ok(())
}

fn disconnect_one(agent: AgentDef) -> Result<(), ConnectError> {
    let home = config_home();
    disconnect_one_to(agent, home.as_deref(), &mut std::io::stdout().lock())
}

/// Disconnects `agent` while writing user-facing progress lines to `out`.
pub(super) fn disconnect_one_to(
    agent: AgentDef,
    home: Option<&Path>,
    out: &mut impl std::io::Write,
) -> Result<(), ConnectError> {
    crate::cli::write_to(
        out,
        format_args!("Disconnecting SlugAudit from {}...\n", agent.display_name),
    )?;
    if agent.manual_steps && matches!(agent.is_connected(home), Ok(true)) {
        crate::cli::write_to(
            out,
            format_args!(
                "{} is configured manually; remove the `slugaudit` entry from {} by hand.\n",
                agent.display_name,
                guidance::target(&agent, home),
            ),
        )?;
        return Ok(());
    }
    match disconnect_agent(&agent, home)? {
        DisconnectOutcome::Disconnected => crate::cli::write_to(out, format_args!("Done.\n"))?,
        DisconnectOutcome::NothingToDo => {
            crate::cli::write_to(out, format_args!("Nothing to disconnect.\n"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
