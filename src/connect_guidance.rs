//! User-facing instructions and provenance for the `connect` surface.
//!
//! Policy for this surface: **tell the user how, rather than leaving them not
//! knowing or breaking it trying.** Concretely, every message built here
//!
//! * names what was or was not changed (a config path, or the exact command
//!   that was run),
//! * carries the paste-ready remedy from the registry when we refused or a
//!   runtime attempt failed, and
//! * ends with the `Reference:` the information comes from (a probed version,
//!   or the docs date) and the vendor `Docs:` page to re-check.
//!
//! The functions are string builders: callers own the I/O so the exact
//! wording stays testable without capturing stdout. `print_lines` is the
//! one writer, and it only emits lines the builders produced.

use crate::cli::ConnectError;
use crate::connect::registry_types::binary_text;
use crate::connect_agents::{AgentDef, Evidence};
use std::path::Path;

/// The config file we would touch, or the client id when no verified path
/// exists.
#[must_use]
pub fn target(agent: &AgentDef, home: Option<&Path>) -> String {
    home.and_then(|home| agent.config.resolve(home))
        .map_or_else(
            || format!("`{}`", agent.id),
            |path| path.display().to_string(),
        )
}

/// `Reference: <version/date>` — always printed, so a probed registration is
/// distinguishable from a documentation-based one.
fn reference_line(agent: &AgentDef) -> String {
    format!("Reference: {}", agent.reference)
}

/// `Docs: <url>` — printed whenever we acted on documentation rather than a
/// probed binary.
fn docs_line(agent: &AgentDef) -> String {
    format!("Docs: {}", agent.docs_url)
}

/// `Reference:` always; `Docs:` too when the entry rests on documentation
/// rather than on a probe of the installed binary.
fn provenance(agent: &AgentDef) -> Vec<String> {
    let mut lines = vec![reference_line(agent)];
    if agent.evidence != Evidence::VerifiedByProbe {
        lines.push(docs_line(agent));
    }
    lines
}

/// The per-OS paths we know about, when the entry is platform-specific and the
/// current platform is not one of them.
fn platform_note(agent: &AgentDef) -> Option<String> {
    let (macos, windows, linux) = agent.config.per_os_paths()?;
    agent.config.resolve(Path::new("")).is_none().then(|| {
        format!(
            "platform `{}` is not one of the documented targets; known paths: \
                 macOS `{macos}`, Windows `{windows}`, Linux `{linux}`",
            std::env::consts::OS
        )
    })
}

/// The paste-ready remedy for a refusal, or an honest pointer when the entry
/// has none.
fn remedy(agent: &AgentDef, binary: &Path) -> String {
    if let Some(remedy) = agent.guidance(binary) {
        return remedy;
    }
    match binary_text(binary) {
        Ok(text) => format!(
            "no verified automated instructions for this client; re-check the docs link below \
             and register `{text}` by hand"
        ),
        Err(error) => error.to_string(),
    }
}

/// What the user should do by hand: the registry remedy when documented, else
/// advice that matches what actually happened. `attempted` is the command a
/// client CLI rejected — a CLI that ran is already installed, so advising an
/// install there would describe work that is done.
fn hand_instructions(agent: &AgentDef, binary: &Path, attempted: Option<&str>) -> String {
    if let Some(remedy) = agent.guidance(binary) {
        return remedy;
    }
    match attempted {
        Some(command) => format!(
            "the client rejected the registration; read the failure above, fix the cause, \
             and re-run `slugaudit connect {}`. The command it rejected was:\n  {command}",
            agent.id
        ),
        None => format!(
            "fix the problem above, then re-run `slugaudit connect {}` (the docs link is below)",
            agent.id
        ),
    }
}

/// The manual advice when the client's own CLI is absent from `PATH`:
/// installing it is the missing step.
fn absent_instructions(agent: &AgentDef, binary: &Path, cli: &str, command: &str) -> String {
    if let Some(remedy) = agent.guidance(binary) {
        return remedy;
    }
    format!(
        "install `{cli}` and re-run `slugaudit connect {}`, or run this yourself:\n  {command}",
        agent.id
    )
}

/// True when the automated path may touch the config file at all: an
/// established shape and a verified path.
#[must_use]
pub fn config_writable(agent: &AgentDef) -> bool {
    agent.writes_config_file() && agent.config.is_usable() && agent.shape.write_section().is_some()
}

/// The refusal error: what we will not attempt, the remedy, and the source.
#[must_use]
pub fn refusal(agent: &AgentDef, binary: &Path, home: Option<&Path>) -> ConnectError {
    let mut reason = format!(
        "no automated write is possible for {}: ",
        target(agent, home),
    );
    match agent.blocker() {
        Some(blocker) => reason.push_str(blocker),
        None => reason.push_str("its registration mechanism could not be established"),
    }
    if let Some(note) = platform_note(agent) {
        reason.push_str(". ");
        reason.push_str(&note);
    }
    reason.push_str(".\n");
    reason.push_str(&remedy(agent, binary));
    push_provenance(&mut reason, agent);
    ConnectError::UnsupportedAgent {
        agent: agent.display_name.to_string(),
        reason,
    }
}

fn push_provenance(reason: &mut String, agent: &AgentDef) {
    // A refusal always ends with both lines: the user is being sent to the
    // vendor page, whether or not the entry came from a probe.
    reason.push('\n');
    reason.push_str(&reference_line(agent));
    reason.push('\n');
    reason.push_str(&docs_line(agent));
}

/// The client's executable is not on PATH, so nothing can be attempted.
#[must_use]
pub fn cli_absent(agent: &AgentDef, binary: &Path, cli: &str, command: &str) -> ConnectError {
    let mut reason = format!(
        "`{cli}` was not found on PATH, so slugaudit cannot register with {} automatically.\n{}",
        agent.display_name,
        absent_instructions(agent, binary, cli, command)
    );
    push_provenance(&mut reason, agent);
    ConnectError::UnsupportedAgent {
        agent: agent.display_name.to_string(),
        reason,
    }
}

/// What to print after a successful registration: the concrete change, then
/// the provenance.
#[must_use]
pub fn success_lines(agent: &AgentDef, action: &str) -> Vec<String> {
    let mut lines = vec![action.to_owned()];
    lines.extend(provenance(agent));
    lines
}

/// What to print when an attempt failed: what we tried, why it failed, how to
/// finish by hand, and the provenance — never a bare error.
pub fn failure_lines(
    agent: &AgentDef,
    binary: &Path,
    attempted: &str,
    hint: Option<&str>,
    why: &str,
) -> Vec<String> {
    let mut reason = format!(
        "slugaudit could not register with {}.\n  Tried: {attempted}\n  Failed: {why}\n  \
         Do it by hand: {}",
        agent.display_name,
        hand_instructions(agent, binary, hint)
    );
    push_provenance(&mut reason, agent);
    reason.split('\n').map(str::to_owned).collect()
}

/// Explains a CLI→file fallback so the user knows which route was taken. `why`
/// is the full clause that follows the client's name, for example
/// `was not found on PATH` or ``rejected `mcp add` (exited with 1)``.
#[must_use]
pub fn fallback_note(agent: &AgentDef, home: Option<&Path>, cli: &str, why: &str) -> String {
    format!(
        "`{cli}` {why}, so slugaudit wrote {} instead.",
        target(agent, home)
    )
}

/// Writes already-built guidance lines to the CLI's stdout. This is the one
/// I/O helper in this module; the wording itself is built and tested above.
pub(crate) fn print_lines(lines: &[String]) -> Result<(), ConnectError> {
    for line in lines {
        crate::cli::out(format_args!("{line}\n"))?;
    }
    Ok(())
}

/// Keeps the failure that caused the output printing: a closed pipe while
/// relaying the client's own output must never replace the real error.
pub(crate) fn keep_source_error(
    source: ConnectError,
    print_result: Result<(), ConnectError>,
) -> ConnectError {
    if let Err(error) = print_result {
        tracing::warn!(
            error = %error,
            "could not print the failure output; keeping the source error"
        );
    }
    source
}

#[cfg(test)]
#[path = "connect_guidance_tests.rs"]
mod tests;
