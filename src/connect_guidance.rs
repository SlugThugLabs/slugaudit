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
//! The functions are pure string builders: callers own the I/O so the exact
//! wording stays testable without capturing stdout.

use crate::cli::ConnectError;
use crate::connect_agents::{AgentDef, Evidence};
use std::path::Path;

/// The config file we would touch, or the client id when no verified path
/// exists.
pub fn target(agent: &AgentDef, home: Option<&Path>) -> String {
    home.and_then(|home| agent.config.resolve(home))
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| format!("`{}`", agent.id))
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
    agent.guidance(binary).unwrap_or_else(|| {
        format!(
            "no verified automated instructions for this client; re-check the docs link below \
             and register `{}` by hand",
            binary.display()
        )
    })
}

/// What the user should do by hand: the registry remedy when documented, else
/// the exact command we would have run, else a pointer to the docs.
fn hand_instructions(agent: &AgentDef, binary: &Path, hint: Option<&str>) -> String {
    if let Some(remedy) = agent.guidance(binary) {
        return remedy;
    }
    match hint {
        Some(command) => format!(
            "install `{}` and re-run `slugaudit connect {}`, or run this yourself:\n  {command}",
            agent.cli.unwrap_or("the client's CLI"),
            agent.id
        ),
        None => format!(
            "fix the problem above, then re-run `slugaudit connect {}` (the docs link is below)",
            agent.id
        ),
    }
}

/// True when the automated path may touch the config file at all: an
/// established shape and a verified path.
pub fn config_writable(agent: &AgentDef) -> bool {
    agent.writes_config_file() && agent.config.is_usable() && agent.shape.write_section().is_some()
}

/// The refusal error: what we will not attempt, the remedy, and the source.
pub fn refusal(agent: &AgentDef, binary: &Path, home: Option<&Path>) -> ConnectError {
    let mut reason = format!(
        "no automated write is possible for {}: {}",
        target(agent, home),
        agent
            .blocker()
            .unwrap_or("its registration mechanism could not be established")
    );
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

/// A documented CLI whose `mcp add` we could not confirm at runtime: refuse
/// rather than guess the syntax.
pub fn unconfirmed_cli(agent: &AgentDef, binary: &Path, cli: &str, command: &str) -> ConnectError {
    let mut reason = format!(
        "`{cli} mcp add` was not confirmed at runtime, so slugaudit will not guess the syntax \
         for a client it has only read about.\n{}",
        hand_instructions(agent, binary, Some(command))
    );
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
pub fn cli_absent(agent: &AgentDef, binary: &Path, cli: &str, command: &str) -> ConnectError {
    let mut reason = format!(
        "`{cli}` was not found on PATH, so slugaudit cannot register with {} automatically.\n{}",
        agent.display_name,
        hand_instructions(agent, binary, Some(command))
    );
    push_provenance(&mut reason, agent);
    ConnectError::UnsupportedAgent {
        agent: agent.display_name.to_string(),
        reason,
    }
}

/// What to print after a successful registration: the concrete change, then
/// the provenance.
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

/// Explains a CLI→file fallback so the user knows which route was taken.
pub fn fallback_note(agent: &AgentDef, home: Option<&Path>, cli: &str, why: &str) -> String {
    format!(
        "`{cli}` was {why}, so slugaudit wrote {} instead.",
        target(agent, home)
    )
}
