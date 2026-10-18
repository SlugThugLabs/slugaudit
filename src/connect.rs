//! Registering or removing this binary as the `slugaudit` MCP server in an AI agent or editor.
#![allow(clippy::print_stdout)]

use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, detected_agents, find_agent};
use crate::connect_exec::{connect_agent, disconnect_agent};
use crate::install::{config_home, running_binary, slugaudit_dir};
use std::io::Write as _;
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

pub(crate) fn prefer_slugthug_binary(current: &Path) -> PathBuf {
    if let Some(slugaudit) = slugaudit_dir() {
        let candidate = slugaudit.join("slugaudit");
        if candidate.exists() {
            return candidate;
        }
    }
    current.to_path_buf()
}

fn connect_one(agent: AgentDef) -> Result<(), ConnectError> {
    let binary = running_binary().map_err(ConnectError::BinaryPath)?;
    let binary = prefer_slugthug_binary(&binary);
    let home = config_home();
    if agent.is_connected(home.as_deref()) {
        println!(
            "SlugAudit is already registered with {}. Updating connection...",
            agent.display_name
        );
    } else {
        println!("Connecting SlugAudit to {}...", agent.display_name);
    }
    connect_agent(&agent, &binary, home.as_deref())?;
    println!("Done.");
    if let Some(cli) = agent.cli {
        println!("Verify with: {} mcp list", cli);
    }
    Ok(())
}

fn disconnect_one(agent: AgentDef) -> Result<(), ConnectError> {
    let home = config_home();
    println!("Disconnecting SlugAudit from {}...", agent.display_name);
    disconnect_agent(&agent, home.as_deref())?;
    println!("Done.");
    Ok(())
}

/// Runs the interactive terminal prompt to choose and connect a detected agent.
pub fn run_connect_interactive() -> Result<(), ConnectError> {
    let home = config_home();
    let detected = detected_agents(home.as_deref());
    if detected.is_empty() {
        println!("No supported AI agents or editors were detected on this machine.");
        println!("Install an agent (e.g. agy, claude, gemini, cursor) or use manual setup.");
        return Ok(());
    }
    println!("Connect SlugAudit to an AI agent or editor:\n");
    for (i, agent) in detected.iter().enumerate() {
        let status = if agent.is_connected(home.as_deref()) {
            " [Connected ✅]"
        } else {
            ""
        };
        println!("  {}) {:<24}{}", i + 1, agent.display_name, status);
    }
    println!();
    print!("Choose an agent [1-{}]: ", detected.len());
    std::io::stdout().flush()?;

    let choice = crate::prompt::read_prompt_line()?.unwrap_or_default();
    let index = parse_choice(&choice, detected.len())?;
    let agent = detected
        .get(index)
        .copied()
        .ok_or(ConnectError::InvalidChoice)?;
    if agent.is_connected(home.as_deref()) {
        print!(
            "{} is already connected. Re-connect / update path? [y/N]: ",
            agent.display_name
        );
        std::io::stdout().flush()?;
        let confirm = crate::prompt::read_prompt_line()?.unwrap_or_default();
        if !crate::prompt::is_yes(&confirm) {
            println!("Skipped.");
            return Ok(());
        }
    }
    connect_one(agent)
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
        println!("No active SlugAudit connections found.");
        return Ok(());
    }
    println!("Connected AI agents and editors:\n");
    for (i, agent) in connected.iter().enumerate() {
        println!("  {}) {}", i + 1, agent.display_name);
    }
    println!();
    print!("Choose an agent to disconnect [1-{}]: ", connected.len());
    std::io::stdout().flush()?;

    let choice = crate::prompt::read_prompt_line()?.unwrap_or_default();
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
