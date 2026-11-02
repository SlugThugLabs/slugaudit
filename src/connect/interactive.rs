//! The interactive pickers behind `slugaudit connect` and `slugaudit
//! disconnect` without an agent name: list the detected or connected agents,
//! read the answer, and hand the chosen agent to the registration code in the
//! parent module.

use super::{connect_one, connection_state, disconnect_one};
use crate::cli;
use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, detected_agents};
use crate::install::config_home;

/// The answer to an interactive prompt; end of input is an error, not an
/// empty answer.
fn read_answer() -> Result<String, ConnectError> {
    answer_from(crate::prompt::read_prompt_line()?)
}

/// Maps a prompt read onto its answer, keeping end of input distinct from an
/// empty line so a closed stdin never reads as an invalid choice.
pub(super) fn answer_from(line: Option<String>) -> Result<String, ConnectError> {
    line.ok_or(ConnectError::EndOfInput)
}

/// Writes an interactive prompt to `out` and flushes it, keeping a closed
/// stdout as the typed I/O error. `out` is a parameter so a failing writer can
/// drive the error arm without a test-only production branch.
pub(super) fn write_prompt(
    out: &mut impl std::io::Write,
    text: std::fmt::Arguments<'_>,
) -> Result<(), ConnectError> {
    cli::write_to(out, text).map_err(ConnectError::Io)
}

/// Runs the interactive terminal prompt to choose and connect a detected agent.
///
/// # Errors
///
/// Fails when writing to the terminal fails, when input ends before an answer
/// or the answer is not a valid choice, or when registering the chosen agent
/// fails.
pub fn run_connect_interactive() -> Result<(), ConnectError> {
    let home = config_home();
    let crate::connect_agents::Detection {
        agents: detected,
        failures,
    } = detected_agents(home.as_deref());
    for failure in &failures {
        cli::err(format_args!("Note: {failure}\n"))?;
    }
    if detected.is_empty() {
        cli::out(format_args!(
            "No supported AI agents or editors were detected on this machine.\n"
        ))?;
        if !failures.is_empty() {
            cli::out(format_args!(
                "Could not check {} client(s); see the notes above.\n",
                failures.len()
            ))?;
        }
        cli::out(format_args!(
            "Install an agent (e.g. agy, claude, gemini, cursor) or use manual setup.\n"
        ))?;
        return Ok(());
    }
    cli::out(format_args!(
        "Connect SlugAudit to an AI agent or editor:\n\n"
    ))?;
    let states = detected
        .iter()
        .map(|agent| connection_state(agent, home.as_deref()))
        .collect::<Result<Vec<bool>, ConnectError>>()?;
    print_detected_agents(&detected, &states)?;
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
    let already_connected = states
        .get(index)
        .copied()
        .ok_or(ConnectError::InvalidChoice)?;
    if already_connected && !confirm_reconnect(agent)? {
        return Ok(());
    }
    connect_one(agent, already_connected)
}

/// Asks whether to update an already-connected agent; returns `false` when the
/// user declines, after printing the skip line.
fn confirm_reconnect(agent: AgentDef) -> Result<bool, ConnectError> {
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

/// Prints the numbered list of detected agents with their connection status;
/// `connected[i]` is the state of `detected[i]`.
fn print_detected_agents(detected: &[AgentDef], connected: &[bool]) -> Result<(), ConnectError> {
    for (i, (agent, is_connected)) in detected.iter().zip(connected).enumerate() {
        let status = if *is_connected {
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
///
/// # Errors
///
/// Fails when writing to the terminal fails, when input ends before an answer
/// or the answer is not a valid choice, or when removing the chosen agent's
/// registration fails.
pub fn run_disconnect_interactive() -> Result<(), ConnectError> {
    let home = config_home();
    let mut connected = Vec::new();
    let mut unchecked = 0usize;
    for agent in crate::connect_agents::AGENTS {
        match agent.is_connected(home.as_deref()) {
            Ok(true) => connected.push(*agent),
            Ok(false) => {}
            Err(failure) => {
                cli::err(format_args!("Note: {failure}\n"))?;
                unchecked += 1;
            }
        }
    }
    if connected.is_empty() {
        cli::out(format_args!("No active SlugAudit connections found.\n"))?;
        if unchecked > 0 {
            cli::out(format_args!(
                "Could not check {unchecked} client(s); see the notes above.\n"
            ))?;
        }
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
pub(super) fn parse_choice(answer: &str, count: usize) -> Result<usize, ConnectError> {
    match answer.trim().parse::<usize>() {
        Ok(choice) if (1..=count).contains(&choice) => Ok(choice - 1),
        _ => Err(ConnectError::InvalidChoice),
    }
}
