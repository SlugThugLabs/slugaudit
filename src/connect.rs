//! Registering this binary as the `slugaudit` MCP server in an AI agent
//! (Claude Code, Grok, Codex). Each agent is driven through its own CLI
//! rather than by editing its config files directly, so the agent owns its
//! own config format and we never corrupt it.
#![allow(clippy::print_stdout)]

use super::cli::{ConnectAgent, ConnectError, slugthug_home};
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};

/// Register the running binary as the `slugaudit` stdio MCP server in the
/// given agent. The binary's own path (`std::env::current_exe()`) is what
/// gets written into the agent's MCP config, so a `cargo install`'d binary
/// keeps working across upgrades automatically.
///
/// If `~/.slugthug/bin/slugaudit-mcp` exists (the user ran `install`), that
/// stable path is registered instead of wherever the binary happens to sit
/// right now — so rebuilding from source later doesn't stale the agent's
/// registration.
///
/// Each agent is registered at its user/global scope so SlugAudit is
/// available in every project rather than only the directory `connect` was
/// run from. The server itself is per-project (each enabled project has
/// its own `.slugaudit/` SQLite index), so a single global registration
/// covers everything.
pub fn run_connect(agent: ConnectAgent) -> Result<(), ConnectError> {
    let binary = std::env::current_exe().map_err(ConnectError::BinaryPath)?;
    let binary = prefer_slugthug_binary(&binary);
    connect_one(agent, &binary)
}

/// If the user has run `install` and `~/.slugthug/bin/slugaudit-mcp`
/// exists, return that path so `connect` registers the stable location
/// rather than a one-off build artifact. Otherwise returns `current`
/// unchanged.
fn prefer_slugthug_binary(current: &Path) -> PathBuf {
    let slugthug = slugthug_home().join("bin").join("slugaudit-mcp");
    if slugthug.exists() {
        slugthug
    } else {
        current.to_path_buf()
    }
}

fn connect_one(agent: ConnectAgent, binary: &Path) -> Result<(), ConnectError> {
    let cli = agent.cli_name();
    if !binary_exists(cli) {
        return Err(ConnectError::AgentMissing {
            agent: agent.display_name().to_string(),
            cli: cli.to_string(),
        });
    }

    println!("Connecting SlugAudit to {}...", agent.display_name());

    remove_existing(agent, cli)?;
    add_server(agent, cli, binary)?;

    println!("Done. Verify with: {} mcp list", cli,);
    Ok(())
}

fn remove_existing(agent: ConnectAgent, cli: &str) -> Result<(), ConnectError> {
    let status = std::process::Command::new(cli)
        .args(["mcp", "remove", "slugaudit"])
        .args(scope_remove_args(agent))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|inner| ConnectError::AgentCommand {
            cli: cli.to_string(),
            inner,
        })?;
    // Exit 0 = removed, non-zero = was not registered. Either is fine.
    if !status.success() {
        tracing::debug!(
            agent = agent.display_name(),
            status = %status,
            "no existing SlugAudit registration to remove"
        );
    }
    Ok(())
}

fn add_server(agent: ConnectAgent, cli: &str, binary: &Path) -> Result<(), ConnectError> {
    let mut cmd = std::process::Command::new(cli);
    cmd.args(["mcp", "add", "slugaudit"]);
    cmd.args(scope_add_args(agent));
    cmd.arg("--");
    cmd.arg(binary);

    let output = cmd.output().map_err(|inner| ConnectError::AgentCommand {
        cli: cli.to_string(),
        inner,
    })?;

    // Surface the agent's own stdout/stderr so the user sees what happened.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        println!("{stdout}");
    }
    if !stderr.is_empty() {
        eprintln!("{stderr}");
    }

    if !output.status.success() {
        return Err(ConnectError::AddFailed {
            cli: cli.to_string(),
            status: output.status.to_string(),
        });
    }
    Ok(())
}

/// Extra args for the `mcp remove` invocation, per agent.
fn scope_remove_args(agent: ConnectAgent) -> &'static [&'static str] {
    match agent {
        // Claude: `mcp remove` requires an explicit scope; `user` matches
        // where `add` writes by default below.
        ConnectAgent::Claude => &["-s", "user"],
        ConnectAgent::Grok => &["--scope", "user"],
        // Codex has no scoped remove — it just deletes the named server.
        ConnectAgent::Codex => &[],
    }
}

/// Extra args for the `mcp add` invocation, per agent.
fn scope_add_args(agent: ConnectAgent) -> &'static [&'static str] {
    match agent {
        // Claude defaults to `local` (project-scoped) but SlugAudit is a
        // global tool — one registration covers every project — so we pin
        // `user` explicitly.
        ConnectAgent::Claude => &["-s", "user"],
        ConnectAgent::Grok => &["--scope", "user"],
        // Codex has no scope flag; it always writes ~/.codex/config.toml.
        ConnectAgent::Codex => &[],
    }
}

fn binary_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Interactive menu: print the supported agents and read a choice from
/// stdin. Used when `connect` is run with no AGENT argument.
pub fn run_connect_interactive() -> Result<(), ConnectError> {
    println!(
        "Connect SlugAudit to an AI agent (registers this binary as the `slugaudit` MCP server):\n"
    );
    for (i, agent) in ConnectAgent::all().iter().enumerate() {
        println!("  {i}) {}  ({})", agent.display_name(), agent.cli_name());
    }
    println!();
    print!("Choose an agent [0-{}]: ", ConnectAgent::all().len() - 1);
    std::io::stdout().flush()?;

    let mut choice = String::new();
    std::io::stdin().lock().read_line(&mut choice)?;
    let choice = choice.trim().parse::<usize>();

    let agent = choice
        .ok()
        .and_then(|i| ConnectAgent::all().get(i).copied())
        .ok_or_else(|| {
            eprintln!("Invalid choice.");
            std::process::exit(1);
        })
        .unwrap();

    connect_one(
        agent,
        &std::env::current_exe().map_err(ConnectError::BinaryPath)?,
    )
}
