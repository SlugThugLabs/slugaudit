//! The registration policy for `connect`/`disconnect`: probe-or-fallback
//! dispatch and client command execution.
//!
//! JSON config-file reads and atomic writes live in
//! `crate::connect::config_file`; the user-facing wording in
//! [`crate::connect_guidance`].
use crate::cli::ConnectError;
use crate::connect::config_file::{remove_json_slugaudit, write_agent_config};
use crate::connect_agents::{AgentDef, Strategy};
use crate::connect_cli;
use crate::connect_guidance as guidance;
use std::path::Path;

pub use crate::connect::config_file::has_slugaudit;

#[cfg(test)]
use crate::connect::config_file::write_atomic;

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
    // Anything we cannot be confident about at call time is instructed, never
    // attempted: unverified mechanisms and entries needing a manual step.
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
/// guidance before returning the error. One unit because every caller treats
/// a failed write as "report why, then fail".
fn write_and_report(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    note: Option<&str>,
) -> Result<(), ConnectError> {
    if let Err(err) = write_agent_config(agent, binary, home, note) {
        report_failure(
            agent,
            binary,
            &write_action(agent, home),
            None,
            &err.to_string(),
        )?;
        return Err(err);
    }
    Ok(())
}

/// The CLI route. Probed entries run the grammar recorded at probe time;
/// documented entries are attempted only when their own binary confirms
/// `mcp add` exists right now.
fn connect_via_cli(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
) -> Result<(), ConnectError> {
    match agent.cli.filter(|cli| crate::probe::cli_on_path(cli)) {
        Some(cli) => run_installed_cli(agent, binary, home, cli),
        None => connect_without_cli(agent, binary, home),
    }
}

/// The client's binary is not on `PATH`. With a documented file path we can
/// still write the config; otherwise the user gets instructions.
fn connect_without_cli(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
) -> Result<(), ConnectError> {
    match (agent.file_fallback, agent.cli) {
        (true, Some(cli)) if guidance::config_writable(agent) => {
            let note = guidance::fallback_note(agent, home, cli, "not found on PATH");
            write_and_report(agent, binary, home, Some(&note))
        }
        (_, Some(cli)) => Err(guidance::cli_absent(
            agent,
            binary,
            cli,
            &connect_cli::display_command(cli, &connect_cli::add_args(agent, binary)?),
        )),
        (_, None) => Err(guidance::refusal(agent, binary, home)),
    }
}

/// Drives the installed client's `mcp add`, falling back to the config file
/// when the entry allows it and the CLI rejects the call.
fn run_installed_cli(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    cli: &str,
) -> Result<(), ConnectError> {
    let args = connect_cli::add_args(agent, binary)?;
    let attempted = connect_cli::display_command(cli, &args);
    match connect_cli::run_add(cli, &args) {
        Ok(()) => {
            guidance::print_lines(&guidance::success_lines(
                agent,
                &format!("Ran: {attempted}"),
            ))?;
            Ok(())
        }
        Err(cli_err) if agent.file_fallback && guidance::config_writable(agent) => {
            // The safer route: leave the CLI alone and merge the config file.
            let note = guidance::fallback_note(agent, home, cli, "rejected `mcp add`");
            match write_agent_config(agent, binary, home, Some(&note)) {
                Ok(()) => Ok(()),
                Err(file_err) => {
                    let why = format!("{cli_err}; the file fallback failed too: {file_err}");
                    report_failure(
                        agent,
                        binary,
                        &write_action(agent, home),
                        Some(&attempted),
                        &why,
                    )?;
                    Err(cli_err)
                }
            }
        }
        Err(cli_err) => {
            report_failure(
                agent,
                binary,
                &attempted,
                Some(&attempted),
                &cli_err.to_string(),
            )?;
            Err(cli_err)
        }
    }
}

fn write_action(agent: &AgentDef, home: Option<&Path>) -> String {
    format!("write {}", guidance::target(agent, home))
}

/// A failed attempt still ends with instructions, the source it came from, and
/// the docs page — never a bare error. A failure to write that guidance is
/// returned rather than dropped.
fn report_failure(
    agent: &AgentDef,
    binary: &Path,
    attempted: &str,
    hint: Option<&str>,
    why: &str,
) -> Result<(), ConnectError> {
    guidance::print_lines(&guidance::failure_lines(
        agent, binary, attempted, hint, why,
    ))
}

/// What a disconnect actually did, so the caller does not report success for
/// a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectOutcome {
    /// At least one registration route was touched.
    Disconnected,
    /// There was no home, config entry, or installed client CLI to remove.
    NothingToDo,
}

/// Removes SlugAudit from one agent: the CLI route when its binary is present,
/// and the file entry only for configs this tool writes.
pub fn disconnect_agent(
    agent: &AgentDef,
    home: Option<&Path>,
) -> Result<DisconnectOutcome, ConnectError> {
    let Some(home) = home else {
        return Ok(DisconnectOutcome::NothingToDo);
    };
    let mut touched = false;
    // Only entries slugaudit actually writes to carry a removable file entry,
    // and only where the written shape is established; detection-only paths
    // (`.claude.json`, `config.yaml`, `config.toml`) are left untouched.
    if agent.writes_config_file()
        && agent.shape.write_section().is_some()
        && let Some(path) = agent.config.resolve(home)
    {
        touched = remove_json_slugaudit(&path, agent.shape.sections())?;
    }
    if matches!(agent.strategy, Strategy::CliDirect)
        && let Some(cli) = agent.cli
        && crate::probe::cli_on_path(cli)
    {
        connect_cli::run_remove(agent, cli)?;
        touched = true;
    }
    Ok(if touched {
        DisconnectOutcome::Disconnected
    } else {
        DisconnectOutcome::NothingToDo
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_agents::find_agent;

    #[test]
    fn an_atomic_write_for_a_path_without_a_parent_reports_the_home_unavailable() {
        let error = write_atomic(Path::new("/"), b"{}").expect_err("root has no parent");

        assert!(
            matches!(error, ConnectError::HomeUnavailable),
            "expected HomeUnavailable, got {error:?}"
        );
    }

    #[test]
    fn disconnecting_without_a_config_home_reports_nothing_to_do() {
        let goose = find_agent("goose").expect("goose is registered");

        let outcome = disconnect_agent(&goose, None).expect("a missing home is not a failure");

        assert_eq!(outcome, DisconnectOutcome::NothingToDo);
    }

    #[test]
    fn disconnecting_an_entry_with_no_writable_section_or_cli_reports_nothing_to_do() {
        let temp = tempfile::tempdir().expect("temp dir");

        for id in ["goose", "crush"] {
            let agent = find_agent(id).expect(id);
            let outcome =
                disconnect_agent(&agent, Some(temp.path())).expect("the no-op must not fail");

            assert_eq!(outcome, DisconnectOutcome::NothingToDo, "{id}");
        }
    }
}
