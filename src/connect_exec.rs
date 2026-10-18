//! Low-level command execution and JSON config file modification for agent connect.
#![allow(clippy::print_stdout)]
// slugaudit-line-exception: approved-by=slugthug; reason=one cohesive concern - the registration policy for 23 clients (probe-or-fallback dispatch, single-key config merging, and the wiring of every refusal/failure into actionable guidance); it reads as one decision table and moving any branch out scatters the policy that is verified entry-by-entry against the registry

use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, Strategy};
use crate::connect_cli;
use crate::connect_guidance as guidance;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// True when the JSON config at `path` carries a `slugaudit` entry under any
/// of `sections`.
pub fn has_slugaudit(path: &Path, keys: &[&str]) -> bool {
    // Advisory probe: an oversized or unreadable config reads as "not
    // registered" here; the write path bounds the same file again and fails
    // explicitly if the cap blocks a real edit (util::read_file_capped).
    crate::util::read_file_capped(path, crate::util::MAX_FOREIGN_TEXT_BYTES).is_ok_and(|c| {
        serde_json::from_str::<serde_json::Value>(&c).is_ok_and(|v| {
            keys.iter()
                .any(|k| v.get(*k).and_then(|s| s.get("slugaudit")).is_some())
        })
    })
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
    // Anything we cannot be confident about at call time is instructed, never
    // attempted: unverified mechanisms and entries needing a manual step.
    if agent.blocker().is_some() {
        return Err(guidance::refusal(agent, binary, home));
    }
    match agent.strategy {
        Strategy::ConfigFile | Strategy::CliInteractiveOnly => {
            write_agent_config(agent, binary, home, None).inspect_err(|err| {
                report_failure(
                    agent,
                    binary,
                    &write_action(agent, home),
                    None,
                    &err.to_string(),
                );
            })
        }
        Strategy::CliDirect => connect_via_cli(agent, binary, home),
        Strategy::Unknown => Err(guidance::refusal(agent, binary, home)),
    }
}

/// The CLI route. Probed entries run the grammar recorded at probe time;
/// documented entries are attempted only when their own binary confirms
/// `mcp add` exists right now.
fn connect_via_cli(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
) -> Result<(), ConnectError> {
    match agent.cli.filter(|cli| which::which(cli).is_ok()) {
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
            write_agent_config(agent, binary, home, Some(&note)).inspect_err(|err| {
                report_failure(
                    agent,
                    binary,
                    &write_action(agent, home),
                    None,
                    &err.to_string(),
                );
            })
        }
        (_, Some(cli)) => Err(guidance::cli_absent(
            agent,
            binary,
            cli,
            &connect_cli::display_command(cli, &connect_cli::add_args(agent, binary)),
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
    let args = connect_cli::add_args(agent, binary);
    let attempted = connect_cli::display_command(cli, &args);
    match connect_cli::run_add(cli, &args) {
        Ok(()) => {
            print_lines(&guidance::success_lines(
                agent,
                &format!("Ran: {attempted}"),
            ));
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
                    );
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
            );
            Err(cli_err)
        }
    }
}

fn write_action(agent: &AgentDef, home: Option<&Path>) -> String {
    format!("write {}", guidance::target(agent, home))
}

/// Writes the `slugaudit` entry in the shape the client reads: exactly one key
/// merged into a file we can parse. Refuses when the path or shape is unknown,
/// and never rewrites a config it could not parse.
fn write_agent_config(
    agent: &AgentDef,
    binary: &Path,
    home: Option<&Path>,
    note: Option<&str>,
) -> Result<(), ConnectError> {
    if !guidance::config_writable(agent) {
        return Err(guidance::refusal(agent, binary, home));
    }
    let home = home.ok_or(ConnectError::HomeUnavailable)?;
    let path = agent
        .config
        .resolve(home)
        .ok_or_else(|| guidance::refusal(agent, binary, Some(home)))?;
    let section = agent
        .shape
        .write_section()
        .ok_or_else(|| guidance::refusal(agent, binary, Some(home)))?;
    write_json_server(&path, section, agent.shape.entry(binary))?;
    if let Some(note) = note {
        println!("{note}");
    }
    let action = format!("Wrote {section}.slugaudit to {}", path.display());
    print_lines(&guidance::success_lines(agent, &action));
    Ok(())
}

/// A failed attempt still ends with instructions, the source it came from, and
/// the docs page — never a bare error.
fn report_failure(agent: &AgentDef, binary: &Path, attempted: &str, hint: Option<&str>, why: &str) {
    print_lines(&guidance::failure_lines(
        agent, binary, attempted, hint, why,
    ));
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), ConnectError> {
    let parent = path.parent().ok_or(ConnectError::HomeUnavailable)?;
    let count = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".slugaudit-tmp-{}-{}", std::process::id(), count));
    fs::write(&temp, content)?;
    if let Err(err) = fs::rename(&temp, path) {
        if let Err(remove_err) = fs::remove_file(&temp) {
            // Deliberate raw path: one-shot diagnostic naming the leftover
            // temp file beside the config it was meant to replace.
            tracing::warn!(error = %remove_err, path = %temp.display(), "failed to remove temporary file after rename error");
        }
        return Err(ConnectError::Io(err));
    }
    Ok(())
}

fn load_json_or_empty(path: &Path) -> Result<serde_json::Value, ConnectError> {
    match crate::util::read_file_capped(path, crate::util::MAX_FOREIGN_TEXT_BYTES) {
        Ok(c) if c.trim().is_empty() => Ok(serde_json::json!({})),
        Ok(c) => serde_json::from_str(&c).map_err(|source| ConnectError::InvalidConfig {
            path: path.to_path_buf(),
            source,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(err) => Err(ConnectError::Io(err)),
    }
}

fn write_json_server(
    path: &Path,
    section: &str,
    val: serde_json::Value,
) -> Result<(), ConnectError> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut root = load_json_or_empty(path)?;
    let map = root
        .as_object_mut()
        .ok_or_else(|| ConnectError::RootNotAnObject {
            path: path.to_path_buf(),
        })?;
    let s = map.entry(section).or_insert_with(|| serde_json::json!({}));
    // An `if let` here meant a section key holding a non-object silently
    // skipped the insert, the file was rewritten unchanged, and the caller
    // still printed "Wrote {section}.slugaudit to {path}". Refusing is the
    // honest answer.
    s.as_object_mut()
        .ok_or_else(|| ConnectError::SectionNotAnObject {
            section: section.to_owned(),
        })?
        .insert("slugaudit".into(), val);
    write_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())
}

/// Removes SlugAudit from one agent: the CLI route when its binary is present,
/// and the file entry only for configs this tool writes.
pub fn disconnect_agent(agent: &AgentDef, home: Option<&Path>) -> Result<(), ConnectError> {
    let Some(home) = home else { return Ok(()) };
    // Only entries slugaudit actually writes to carry a removable file entry,
    // and only where the written shape is established; detection-only paths
    // (`.claude.json`, `config.yaml`, `config.toml`) are left untouched.
    if agent.writes_config_file()
        && agent.shape.write_section().is_some()
        && let Some(path) = agent.config.resolve(home)
    {
        remove_json_slugaudit(&path, agent.shape.sections())?;
    }
    if matches!(agent.strategy, Strategy::CliDirect)
        && let Some(cli) = agent.cli
        && which::which(cli).is_ok()
    {
        connect_cli::run_remove(agent, cli)?;
    }
    Ok(())
}

fn remove_json_slugaudit(path: &Path, keys: &[&str]) -> Result<(), ConnectError> {
    if path.exists() {
        let mut root = load_json_or_empty(path)?;
        for key in keys {
            if let Some(s) = root.get_mut(*key).and_then(|s| s.as_object_mut()) {
                s.remove("slugaudit");
            }
        }
        write_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    }
    Ok(())
}
