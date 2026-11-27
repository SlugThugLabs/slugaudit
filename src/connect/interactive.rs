//! Interactive `slugaudit connect` and `slugaudit disconnect` pickers and
//! shared numbered-choice prompt primitives.

use super::agent_probe::{
    Detection, DisconnectCandidate, DisconnectScan, ProbeFailure, detected_agents,
    scan_disconnect_candidates,
};
use super::registry_types::AgentDef;
use super::{ConnectError, connect_one, connection_state, disconnect_one_to, guidance};
use crate::cli;
use crate::install::config_home;
use std::path::Path;

/// Maps a prompt read onto its answer, keeping end of input distinct from an
/// empty line so a closed stdin never reads as an invalid choice.
pub(super) fn answer_from(line: Option<String>) -> Result<String, ConnectError> {
    line.ok_or(ConnectError::EndOfInput)
}

/// Writes an interactive prompt to `out` and flushes it, keeping a closed
/// stdout as the typed I/O error.
pub(super) fn write_prompt(
    out: &mut impl std::io::Write,
    text: std::fmt::Arguments<'_>,
) -> Result<(), ConnectError> {
    Ok(cli::write_to(out, text)?)
}

fn prompt_line(
    out: &mut impl std::io::Write,
    read_line: &mut impl FnMut() -> std::io::Result<Option<String>>,
    prompt: std::fmt::Arguments<'_>,
) -> Result<String, ConnectError> {
    write_prompt(out, prompt)?;
    answer_from(read_line()?)
}

pub(super) fn write_check_failures(
    err: &mut impl std::io::Write,
    failures: &[ProbeFailure],
) -> Result<(), ConnectError> {
    for failure in failures {
        cli::write_to(err, format_args!("Note: {failure}\n"))?;
    }
    Ok(())
}

pub(super) fn write_unchecked_summary(
    out: &mut impl std::io::Write,
    count: usize,
) -> Result<(), ConnectError> {
    if count > 0 {
        cli::write_to(
            out,
            format_args!("Could not check {count} client(s); see the notes above.\n"),
        )?;
    }
    Ok(())
}

/// Renders `heading` and a numbered option list on `out`, prompts with `prompt`,
/// reads one answer from `read_line`, and returns the chosen item.
pub(super) fn choose_from<T: Copy>(
    out: &mut impl std::io::Write,
    read_line: &mut impl FnMut() -> std::io::Result<Option<String>>,
    heading: &str,
    items: &[T],
    prompt: &str,
    mut format_row: impl FnMut(T) -> String,
) -> Result<T, ConnectError> {
    cli::write_to(out, format_args!("{heading}\n\n"))?;
    for (number, item) in (1usize..).zip(items.iter().copied()) {
        cli::write_to(out, format_args!("  {number}) {}\n", format_row(item)))?;
    }
    cli::write_to(out, format_args!("\n"))?;
    let choice = prompt_line(
        out,
        read_line,
        format_args!("{prompt} [1-{}]: ", items.len()),
    )?;
    let index = parse_choice(&choice, items.len())?;
    items.get(index).copied().ok_or(ConnectError::InvalidChoice)
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
    let Detection {
        agents: detected,
        failures,
    } = detected_agents(home.as_deref());
    write_check_failures(&mut std::io::stderr().lock(), &failures)?;
    if detected.is_empty() {
        let mut out = std::io::stdout().lock();
        cli::write_to(
            &mut out,
            format_args!("No supported AI agents or editors were detected on this machine.\n"),
        )?;
        write_unchecked_summary(&mut out, failures.len())?;
        cli::write_to(
            &mut out,
            format_args!(
                "Install an agent (e.g. agy, claude, gemini, cursor) or use manual setup.\n"
            ),
        )?;
        return Ok(());
    }
    let entries = detected
        .iter()
        .copied()
        .map(|agent| connection_state(&agent, home.as_deref()).map(|state| (agent, state)))
        .collect::<Result<Vec<(AgentDef, bool)>, ConnectError>>()?;
    let (agent, already_connected) = {
        let mut out = std::io::stdout().lock();
        choose_from(
            &mut out,
            &mut crate::prompt::read_prompt_line,
            "Connect SlugAudit to an AI agent or editor:",
            &entries,
            "Choose an agent",
            |(agent, is_connected)| {
                let status = if is_connected { " [Connected ✅]" } else { "" };
                format!("{:<24}{status}", agent.display_name)
            },
        )?
    };
    if already_connected && !confirm_reconnect(agent)? {
        return Ok(());
    }
    connect_one(agent, home.as_deref(), already_connected)
}

/// Asks whether to update an already-connected agent; returns `false` when the
/// user declines, after printing the skip line.
fn confirm_reconnect(agent: AgentDef) -> Result<bool, ConnectError> {
    let confirm = prompt_line(
        &mut std::io::stdout().lock(),
        &mut crate::prompt::read_prompt_line,
        format_args!(
            "{} is already connected. Re-connect / update path? [y/N]: ",
            agent.display_name
        ),
    )?;
    if crate::prompt::is_yes(&confirm) {
        return Ok(true);
    }
    cli::out(format_args!("Skipped.\n"))?;
    Ok(false)
}

/// The zero-based index of a 1-based menu answer among `count` options, or
/// `InvalidChoice` for anything that is not one of the listed numbers.
pub(super) fn parse_choice(answer: &str, count: usize) -> Result<usize, ConnectError> {
    match answer.trim().parse::<usize>() {
        Ok(choice) if choice <= count => choice.checked_sub(1).ok_or(ConnectError::InvalidChoice),
        _ => Err(ConnectError::InvalidChoice),
    }
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
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    disconnect_interactive_with(
        home.as_deref(),
        &mut stdout,
        &mut stderr,
        &mut crate::prompt::read_prompt_line,
    )
}

/// Stream-injected implementation of [`run_disconnect_interactive`] so every
/// candidate, manual-only, and error branch can be tested deterministically.
fn disconnect_interactive_with(
    home: Option<&Path>,
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
    read_line: &mut impl FnMut() -> std::io::Result<Option<String>>,
) -> Result<(), ConnectError> {
    let DisconnectScan {
        candidates,
        manual_only,
        unchecked,
    } = scan_disconnect_candidates(home);
    write_check_failures(err, &unchecked)?;

    for agent in &manual_only {
        cli::write_to(
            out,
            format_args!(
                "Note: {} is configured manually; remove the `slugaudit` entry from {} by hand.\n",
                agent.display_name,
                guidance::target(agent, home),
            ),
        )?;
    }

    if candidates.is_empty() {
        if manual_only.is_empty() {
            cli::write_to(
                out,
                format_args!("No active SlugAudit connections found.\n"),
            )?;
        }
        write_unchecked_summary(out, unchecked.len())?;
        return Ok(());
    }

    let chosen = choose_from(
        out,
        read_line,
        "Connected AI agents and editors:",
        &candidates,
        "Choose an agent to disconnect",
        |candidate| match candidate {
            DisconnectCandidate::Connected(agent) => agent.display_name.to_owned(),
            DisconnectCandidate::CliManaged(agent) => {
                format!("{} [CLI installed]", agent.display_name)
            }
        },
    )?;
    let (DisconnectCandidate::Connected(agent) | DisconnectCandidate::CliManaged(agent)) = chosen;
    disconnect_one_to(agent, home, out)
}

#[cfg(test)]
#[path = "interactive_tests.rs"]
mod tests;
