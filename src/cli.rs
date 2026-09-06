//! The `slugaudit` binary's command-line surface: `serve` (the MCP server,
//! the default with no arguments). `connect` registers the running binary
//! as a stdio MCP server named `slugaudit` in an AI agent or editor.
//! `disconnect` removes it. `install` copies the binary to a stable path.
#![allow(clippy::print_stdout)]

use crate::util::Style;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Connect { agent: Option<String> },
    Disconnect { agent: Option<String> },
    Install,
    Menu,
    Update,
    Version,
    Help,
}

#[must_use = "the returned Command must be dispatched by the caller"]
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Ok(Command::Serve);
    };
    match first.as_str() {
        "serve" => Ok(Command::Serve),
        "help" => Ok(Command::Help),
        "connect" => Ok(Command::Connect { agent: args.next() }),
        "disconnect" | "remove" => Ok(Command::Disconnect { agent: args.next() }),
        "install" => Ok(Command::Install),
        "menu" => Ok(Command::Menu),
        "update" => Ok(Command::Update),
        "version" | "--version" | "-V" => Ok(Command::Version),
        "--help" | "-h" => Ok(Command::Help),
        other => Err(format!(
            "unknown command {other:?}; expected one of: serve, connect, disconnect, install, update, menu, version, help"
        )),
    }
}

pub const USAGE: &str = "\
slugaudit — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit                        Run the MCP server (stdio transport)
    slugaudit menu                   Interactive setup menu (recommended entry point)
    slugaudit install                Copy binary to ~/.slugthug/slugaudit/
    slugaudit connect [AGENT]        Connect to an AI agent or editor
    slugaudit disconnect [AGENT]     Disconnect from an AI agent or editor
    slugaudit update                 Fetch the latest release and replace this binary
    slugaudit version                Print version (also --version, -V)
    slugaudit help                   Show this message (also --help, -h)

COMMANDS:
    (no command)  Start the MCP server. This is the default and the way
                  your AI agent launches SlugAudit; it speaks JSON-RPC
                  over stdio and blocks until the host closes the pipe.
    menu          Interactive setup: install the binary, connect to an AI
                  agent, get a config snippet for any other MCP client,
                  or run the server for testing. Recommended for first-time setup.
    connect       Register this binary as the `slugaudit` MCP server in
                  an AI agent or editor. With an agent name (e.g. `agy`,
                  `claude`, `gemini`, `cursor`) it connects directly; without
                  one it shows detected installed agents.
    disconnect    Remove SlugAudit registration from an AI agent or editor.
                  Alias: `remove`. With an agent name it disconnects directly;
                  without one it lists connected agents to pick from.
    install       Copy the binary to ~/.slugthug/slugaudit/slugaudit so
                  agents and MCP clients can launch a stable path that
                  survives rebuilds.
    update        Check the latest GitHub release and, if newer than the
                  running binary, download it and atomically replace this
                  binary in place. Uses curl; verifies the sha256 checksum
                  before touching the current executable.
    version       Print the version. Also --version, -V.
    help          Show this message. Also --help, -h.

OPTIONS:
    -h, --help      Show this message.
    -V, --version   Print the version.

EXAMPLES:
    slugaudit menu                   Set everything up interactively
    slugaudit install                Install to ~/.slugthug/slugaudit/
    slugaudit connect agy            Register with Antigravity
    slugaudit connect claude         Register with Claude Code
    slugaudit disconnect claude      Remove registration from Claude Code
    slugaudit update                 Update to the latest release
    slugaudit                        Run the server for your AI agent
";

pub fn usage() -> String {
    let style = Style::stdout();
    if !style.enabled() {
        return USAGE.to_owned();
    }
    let headings = ["USAGE:", "COMMANDS:", "OPTIONS:", "EXAMPLES:"];
    let mut out = String::with_capacity(USAGE.len() + 64);
    for (i, line) in USAGE.lines().enumerate() {
        let trimmed = line.trim_end();
        if i == 0 {
            out.push_str(&style.bold(trimmed));
        } else if headings.contains(&trimmed) {
            out.push_str(&style.bold(&style.cyan(trimmed)));
        } else {
            out.push_str(trimmed);
        }
        out.push('\n');
    }
    out
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("invalid agent choice; choose one of the listed options")]
    InvalidChoice,
    #[error("binary path unavailable: {0}")]
    BinaryPath(std::io::Error),
    #[error("home directory is unavailable")]
    HomeUnavailable,
    #[error("unknown agent {0:?}; run `slugaudit connect` to see detected agents")]
    UnknownAgent(String),
    #[error("{agent} CLI ({cli}) not found on PATH — install it first")]
    AgentMissing { agent: String, cli: String },
    #[error("failed to run `{cli}`: {inner}")]
    AgentCommand { cli: String, inner: std::io::Error },
    #[error("`{cli} mcp remove` exited with {status}")]
    RemoveFailed { cli: String, status: String },
    #[error("`{cli} mcp add` exited with {status} — see the output above")]
    AddFailed { cli: String, status: String },
    #[error(
        "cannot modify {path}: configuration file exists but contains invalid JSON: {source} (fix syntax/comments or remove the file to reset, then retry)"
    )]
    InvalidConfig {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("JSON config error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
