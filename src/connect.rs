//! Registering or removing this binary as the `slugaudit` MCP server in an AI agent or editor.

pub(crate) mod config_file;
pub(crate) mod registry_types;

use crate::cli;
use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, detected_agents, find_agent};
use crate::connect_exec::{DisconnectOutcome, connect_agent, disconnect_agent};
use crate::install::{config_home, running_binary, slugaudit_dir};
use std::path::{Path, PathBuf};

/// Registers SlugAudit as an MCP server for the named agent, or prompts interactively.
pub fn run_connect(agent_name: Option<&str>) -> Result<(), ConnectError> {
    match agent_name {
        Some(name) => {
            let agent =
                find_agent(name).ok_or_else(|| ConnectError::UnknownAgent(name.to_string()))?;
            connect_one(agent)
        }
        None => run_connect_interactive(),
    }
}

/// Disconnects SlugAudit from the named agent, or prompts interactively.
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

/// The answer to an interactive prompt; end of input is an error, not an
/// empty answer.
fn read_answer() -> Result<String, ConnectError> {
    answer_from(crate::prompt::read_prompt_line()?)
}

/// Maps a prompt read onto its answer, keeping end of input distinct from an
/// empty line so a closed stdin never reads as an invalid choice.
fn answer_from(line: Option<String>) -> Result<String, ConnectError> {
    line.ok_or(ConnectError::EndOfInput)
}

/// Maps the running-binary lookup onto the connect error, keeping the I/O
/// cause. Split from [`connect_one`] so the failure arm is testable without
/// faking the process that owns the executable path.
fn binary_path(result: Result<PathBuf, std::io::Error>) -> Result<PathBuf, ConnectError> {
    result.map_err(ConnectError::BinaryPath)
}

/// Writes an interactive prompt to `out` and flushes it, keeping a closed
/// stdout as the typed I/O error. `out` is a parameter so a failing writer can
/// drive the error arm without a test-only production branch.
fn write_prompt(
    out: &mut impl std::io::Write,
    text: std::fmt::Arguments<'_>,
) -> Result<(), ConnectError> {
    cli::write_to(out, text).map_err(ConnectError::Io)
}

fn connect_one(agent: AgentDef) -> Result<(), ConnectError> {
    let binary = binary_path(running_binary())?;
    let binary = prefer_slugthug_binary(&binary)?;
    let home = config_home();
    if agent.is_connected(home.as_deref()) {
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

/// Runs the interactive terminal prompt to choose and connect a detected agent.
pub fn run_connect_interactive() -> Result<(), ConnectError> {
    let home = config_home();
    let detected = detected_agents(home.as_deref());
    if detected.is_empty() {
        cli::out(format_args!(
            "No supported AI agents or editors were detected on this machine.\n"
        ))?;
        cli::out(format_args!(
            "Install an agent (e.g. agy, claude, gemini, cursor) or use manual setup.\n"
        ))?;
        return Ok(());
    }
    cli::out(format_args!(
        "Connect SlugAudit to an AI agent or editor:\n\n"
    ))?;
    print_detected_agents(&detected, home.as_deref())?;
    cli::out(format_args!("\n"))?;
    write_prompt(
        &mut std::io::stdout(),
        format_args!("Choose an agent [1-{}]: ", detected.len()),
    )?;

    let choice = read_answer()?;
    let index = parse_choice(&choice, detected.len())?;
    let agent = detected
        .get(index)
        .copied()
        .ok_or(ConnectError::InvalidChoice)?;
    if !confirm_reconnect(agent, home.as_deref())? {
        return Ok(());
    }
    connect_one(agent)
}

/// Asks whether to update an already-connected agent; returns `false` when the
/// user declines, after printing the skip line.
fn confirm_reconnect(agent: AgentDef, home: Option<&Path>) -> Result<bool, ConnectError> {
    if !agent.is_connected(home) {
        return Ok(true);
    }
    write_prompt(
        &mut std::io::stdout(),
        format_args!(
            "{} is already connected. Re-connect / update path? [y/N]: ",
            agent.display_name
        ),
    )?;
    let confirm = read_answer()?;
    if crate::prompt::is_yes(&confirm) {
        return Ok(true);
    }
    cli::out(format_args!("Skipped.\n"))?;
    Ok(false)
}

/// Prints the numbered list of detected agents with their connection status.
fn print_detected_agents(detected: &[AgentDef], home: Option<&Path>) -> Result<(), ConnectError> {
    for (i, agent) in detected.iter().enumerate() {
        let status = if agent.is_connected(home) {
            " [Connected ✅]"
        } else {
            ""
        };
        cli::out(format_args!(
            "  {}) {:<24}{}\n",
            i + 1,
            agent.display_name,
            status
        ))?;
    }
    Ok(())
}

/// Runs the interactive terminal prompt to choose and disconnect an active agent connection.
pub fn run_disconnect_interactive() -> Result<(), ConnectError> {
    let home = config_home();
    let connected: Vec<_> = crate::connect_agents::AGENTS
        .iter()
        .copied()
        .filter(|a| a.is_connected(home.as_deref()))
        .collect();
    if connected.is_empty() {
        cli::out(format_args!("No active SlugAudit connections found.\n"))?;
        return Ok(());
    }
    cli::out(format_args!("Connected AI agents and editors:\n\n"))?;
    for (i, agent) in connected.iter().enumerate() {
        cli::out(format_args!("  {}) {}\n", i + 1, agent.display_name))?;
    }
    cli::out(format_args!("\n"))?;
    write_prompt(
        &mut std::io::stdout(),
        format_args!("Choose an agent to disconnect [1-{}]: ", connected.len()),
    )?;

    let choice = read_answer()?;
    let index = parse_choice(&choice, connected.len())?;
    let agent = connected
        .get(index)
        .copied()
        .ok_or(ConnectError::InvalidChoice)?;
    disconnect_one(agent)
}

/// The zero-based index of a 1-based menu answer among `count` options, or
/// `InvalidChoice` for anything that is not one of the listed numbers.
fn parse_choice(answer: &str, count: usize) -> Result<usize, ConnectError> {
    match answer.trim().parse::<usize>() {
        Ok(choice) if (1..=count).contains(&choice) => Ok(choice - 1),
        _ => Err(ConnectError::InvalidChoice),
    }
}

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
