//! The `slugaudit-mcp-rust` binary's command-line surface: `serve` (the
//! MCP server, the default with no arguments) and `enable`/`disable` — the
//! one human-facing control described in `ARCHITECTURE.md`, exposed here
//! as real commands instead of a "go create this directory yourself"
//! workaround. `enable` also runs the project's first import immediately,
//! before returning, rather than waiting for an AI to make the first tool
//! call.
//!
//! `connect` registers the running binary as a stdio MCP server named
//! `slugaudit` in a supported AI agent (Claude Code, Grok, or Codex) so
//! the agent can reach SlugAudit's tools without the user hand-writing
//! config. With no argument it presents an interactive menu; pass an agent
//! name to connect it directly.
use crate::project::{self, ProjectRoot};
use crate::{parse, store, sync};
use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Enable { path: PathBuf },
    Disable { path: PathBuf, assume_yes: bool },
    Connect { agent: Option<ConnectAgent> },
    Install,
    Help,
}

/// The AI agents SlugAudit knows how to register itself with. Each variant
/// maps to a CLI tool on PATH (`claude`, `grok`, `codex`) and a known
/// `mcp add` invocation shape. New agents are added here and in
/// `ConnectAgent::all()` — the interactive menu picks them up automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectAgent {
    Claude,
    Grok,
    Codex,
}

impl ConnectAgent {
    pub fn all() -> &'static [ConnectAgent] {
        &[
            ConnectAgent::Claude,
            ConnectAgent::Grok,
            ConnectAgent::Codex,
        ]
    }

    fn cli_name(self) -> &'static str {
        match self {
            ConnectAgent::Claude => "claude",
            ConnectAgent::Grok => "grok",
            ConnectAgent::Codex => "codex",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            ConnectAgent::Claude => "Claude Code",
            ConnectAgent::Grok => "Grok",
            ConnectAgent::Codex => "Codex",
        }
    }
}

impl std::str::FromStr for ConnectAgent {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Ok(ConnectAgent::Claude),
            "grok" => Ok(ConnectAgent::Grok),
            "codex" => Ok(ConnectAgent::Codex),
            other => Err(format!(
                "unknown agent {other:?}; expected one of: claude, grok, codex"
            )),
        }
    }
}

/// Parses the process arguments into a `Command`. Returns `Err` with a
/// human-readable message when the input can't be mapped to a command —
/// currently only an unknown agent name passed to `connect`. The caller
/// (main) prints the message to stderr and exits non-zero; keeping the
/// exit out of this function makes the parser testable.
#[must_use = "the returned Command must be dispatched by the caller"]
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Ok(Command::Serve);
    };
    match first.as_str() {
        "serve" => Ok(Command::Serve),
        "enable" => Ok(Command::Enable {
            path: args
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from),
        }),
        "disable" => {
            let mut path = None;
            let mut assume_yes = false;
            for arg in args {
                if arg == "-y" || arg == "--yes" {
                    assume_yes = true;
                } else {
                    path = Some(PathBuf::from(arg));
                }
            }
            Ok(Command::Disable {
                path: path.unwrap_or_else(|| PathBuf::from(".")),
                assume_yes,
            })
        }
        "connect" => {
            let agent = args
                .next()
                .map(|s| ConnectAgent::from_str(&s))
                .transpose()?;
            Ok(Command::Connect { agent })
        }
        "install" => Ok(Command::Install),
        _ => Ok(Command::Help),
    }
}

pub const USAGE: &str = "\
slugaudit-mcp — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit-mcp                    Run the MCP server (stdio transport)
    slugaudit-mcp serve              Same as running with no arguments
    slugaudit-mcp enable [PATH]      Turn SlugAudit on for PATH (default: .)
                                      and run its first import immediately
    slugaudit-mcp disable [PATH]     Turn SlugAudit off for PATH (default: .),
                                      deleting its database, findings, and evidence
        -y, --yes                    Skip the confirmation prompt
    slugaudit-mcp connect [AGENT]    Register this binary as the `slugaudit`
                                      MCP server in an AI agent. AGENT is one
                                      of: claude, grok, codex. Omit to pick
                                      from an interactive menu.
    slugaudit-mcp install            Copy this binary to ~/.slugthug/bin/ so
                                      it's on a stable path shared with future
                                      slug-branded products.
    slugaudit-mcp help               Show this message
";

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Root(#[from] project::RootError),
    #[error(transparent)]
    Activation(#[from] project::ActivationError),
    #[error(transparent)]
    Store(#[from] store::StoreError),
    #[error(transparent)]
    Publish(#[from] sync::PublishError),
    #[error("failed to read from the terminal: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors from `connect`. Split out from `CliError` because connect has its
/// own failure surface (agent CLI missing, registration command failed)
/// that benefits from distinct, actionable messages.
#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("binary path unavailable: {0}")]
    BinaryPath(std::io::Error),
    #[error("{agent} CLI ({cli}) not found on PATH — install it first")]
    AgentMissing { agent: String, cli: String },
    #[error("failed to run `{cli}`: {inner}")]
    AgentCommand { cli: String, inner: std::io::Error },
    #[error("`{cli} mcp remove` exited with {status}")]
    RemoveFailed { cli: String, status: String },
    #[error("`{cli} mcp add` exited with {status} — see the output above")]
    AddFailed { cli: String, status: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

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

/// The shared home directory for all slug-branded products on this machine:
/// `~/.slugthug/`. Binaries live under `bin/`, per-product data under
/// `<product>/`, and an optional top-level `config.toml` covers shared
/// PostgreSQL config for any product that needs it.
pub fn slugthug_home() -> PathBuf {
    std::env::var_os("SLUGTHUG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".slugthug"))
        })
        .expect("HOME must be set")
}

/// Errors from `install`.
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("could not locate the running binary: {0}")]
    CurrentExe(std::io::Error),
    #[error("could not create {path}: {inner}")]
    Mkdir { path: String, inner: std::io::Error },
    #[error("could not copy the binary to {path}: {inner}")]
    Copy { path: String, inner: std::io::Error },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Copy the running binary into `~/.slugthug/bin/slugaudit-mcp`. Future
/// slug-branded products (codeguard, the rebranded ClauRust, etc.) share
/// the same `~/.slugthug/bin/` directory, so a user who adds it to PATH
/// gets all of them in one go. `connect` registers this stable path in the
/// agent's MCP config, so the agent keeps working across upgrades — just
/// re-run `install` after building a new binary.
pub fn run_install() -> Result<(), InstallError> {
    let source = std::env::current_exe().map_err(InstallError::CurrentExe)?;
    let bin_dir = slugthug_home().join("bin");
    let target = bin_dir.join("slugaudit-mcp");

    std::fs::create_dir_all(&bin_dir).map_err(|inner| InstallError::Mkdir {
        path: bin_dir.display().to_string(),
        inner,
    })?;

    std::fs::copy(&source, &target).map_err(|inner| InstallError::Copy {
        path: target.display().to_string(),
        inner,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms)?;
    }

    println!("Installed slugaudit-mcp to {}", target.display());
    println!(
        "Add {} to your PATH, then run: slugaudit-mcp connect",
        bin_dir.display()
    );
    Ok(())
}

/// # Errors
///
/// Returns an error if `path` doesn't resolve to a real directory, if the
/// activation marker can't be created, or if the initial import fails.
pub fn run_enable(path: &Path) -> Result<(), CliError> {
    let root = ProjectRoot::resolve(path)?;
    project::enable(&root)?;
    println!("SlugAudit enabled for {}", root.as_path().display());
    println!("Running initial import...");

    let database_path = project::database_path(&root);
    let mut connection = store::open_read_write(&database_path)?;
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION)?;
    println!(
        "Initial import complete: {} file(s) added, {} unchanged (revision {})",
        report.added, report.unchanged, report.revision_id
    );
    Ok(())
}

/// # Errors
///
/// Returns an error if `path` doesn't resolve to a real directory, if
/// reading the confirmation prompt fails, or if removing the activation
/// marker fails.
pub fn run_disable(path: &Path, assume_yes: bool) -> Result<(), CliError> {
    disable_with_input(path, assume_yes, std::io::stdin().lock())
}

fn disable_with_input(path: &Path, assume_yes: bool, input: impl BufRead) -> Result<(), CliError> {
    let root = ProjectRoot::resolve(path)?;
    let activation = project::activation_dir(&root);
    if !activation.exists() {
        println!("SlugAudit is not enabled for {}", root.as_path().display());
        return Ok(());
    }
    if !assume_yes {
        let prompt = format!(
            "This will permanently delete {} (database, findings, evidence). Continue? [y/N] ",
            activation.display()
        );
        if !confirm(&prompt, input)? {
            println!("Cancelled.");
            return Ok(());
        }
    }
    project::disable(&root)?;
    println!("SlugAudit disabled for {}", root.as_path().display());
    Ok(())
}

fn confirm(prompt: &str, mut input: impl BufRead) -> Result<bool, std::io::Error> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
