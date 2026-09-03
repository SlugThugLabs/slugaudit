//! The `slugaudit-mcp-rust` binary's command-line surface: `serve` (the
//! MCP server, the default with no arguments). `connect` registers the
//! running binary as a stdio MCP server named `slugaudit` in a supported AI
//! agent (Bob, Claude Code, Grok, or Codex). `install` copies the binary to a
//! stable path so it survives rebuilds.
#![allow(clippy::print_stdout)]

use crate::util::Style;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Connect { agent: Option<ConnectAgent> },
    Install,
    Menu,
    Update,
    Version,
    Help,
}

/// The AI agents SlugAudit knows how to register itself with. Each variant
/// maps to a CLI tool on PATH (`claude`, `grok`, `codex`) and a known
/// `mcp add` invocation shape. New agents are added here and in
/// `ConnectAgent::all()` — the interactive menu picks them up automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectAgent {
    Bob,
    Claude,
    Grok,
    Codex,
}

impl ConnectAgent {
    pub fn all() -> &'static [ConnectAgent] {
        &[
            ConnectAgent::Bob,
            ConnectAgent::Claude,
            ConnectAgent::Grok,
            ConnectAgent::Codex,
        ]
    }

    pub(crate) fn cli_name(self) -> &'static str {
        match self {
            ConnectAgent::Bob => "bob",
            ConnectAgent::Claude => "claude",
            ConnectAgent::Grok => "grok",
            ConnectAgent::Codex => "codex",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            ConnectAgent::Bob => "Bob",
            ConnectAgent::Claude => "Claude Code",
            ConnectAgent::Grok => "Grok",
            ConnectAgent::Codex => "Codex",
        }
    }
}

impl FromStr for ConnectAgent {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "bob" => Ok(ConnectAgent::Bob),
            "claude" | "claude-code" | "claude_code" => Ok(ConnectAgent::Claude),
            "grok" => Ok(ConnectAgent::Grok),
            "codex" => Ok(ConnectAgent::Codex),
            other => Err(format!(
                "unknown agent {other:?}; expected one of: bob, claude, grok, codex"
            )),
        }
    }
}

/// Parses the process arguments into a `Command`. Returns `Err` with a
/// human-readable message when the input can't be mapped to a command —
/// an unrecognized first argument, or an unknown agent name passed to
/// `connect`. The caller (`main`) prints the message to stderr and exits
/// non-zero; keeping the exit out of this function makes the parser
/// testable.
///
/// Important: an unknown first argument is **an error**, not a help
/// invocation. A `Command::Help` is reserved for an explicit `"help"`
/// command and for the empty-argument case (where the caller would
/// arguably want help too, but launching the MCP server is the
/// documented default behavior — `serve` and the empty arg are the
/// same). User-typo'd commands should be reported, not silently
/// downgraded to a help screen, to avoid surprises in pipes and CI.
#[must_use = "the returned Command must be dispatched by the caller"]
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Ok(Command::Serve);
    };
    match first.as_str() {
        "serve" => Ok(Command::Serve),
        "help" => Ok(Command::Help),
        "connect" => {
            let agent = args
                .next()
                .map(|s| ConnectAgent::from_str(&s))
                .transpose()?;
            Ok(Command::Connect { agent })
        }
        "install" => Ok(Command::Install),
        "menu" => Ok(Command::Menu),
        "update" => Ok(Command::Update),
        // `version` in all three common spellings so a released binary can
        // be verified against its checksum/tag — `--version` currently
        // errors as an unknown command without this.
        "version" | "--version" | "-V" => Ok(Command::Version),
        "--help" | "-h" => Ok(Command::Help),
        other => Err(format!(
            "unknown command {other:?}; expected one of: serve, connect, install, update, menu, version, help"
        )),
    }
}

pub const USAGE: &str = "\
slugaudit-mcp — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit-mcp                    Run the MCP server (stdio transport)
    slugaudit-mcp menu               Interactive setup menu (recommended entry point)
    slugaudit-mcp install            Copy binary to ~/.slugthug/bin/
    slugaudit-mcp update             Fetch the latest release and replace this binary
    slugaudit-mcp version            Print version (also --version, -V)
    slugaudit-mcp help               Show this message (also --help, -h)

COMMANDS:
    (no command)  Start the MCP server. This is the default and the way
                  your AI agent launches SlugAudit; it speaks JSON-RPC
                  over stdio and blocks until the host closes the pipe.
    menu          Interactive setup: install the binary, connect to an AI
                  agent (Bob, Claude Code, Grok, Codex), get a config
                  snippet for any other MCP client, or run the server
                  for testing. Recommended for first-time setup.
    connect       Register this binary as the `slugaudit` MCP server in
                  an AI agent. With an agent name (`bob`, `claude`,
                  `grok`, `codex`) it connects directly; without one it
                  shows an interactive agent picker.
    install       Copy the binary to ~/.slugthug/bin/slugaudit-mcp so
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
    slugaudit-mcp menu               Set everything up interactively
    slugaudit-mcp install            Install to ~/.slugthug/bin/
    slugaudit-mcp connect bob        Register with Bob
    slugaudit-mcp update             Update to the latest release
    slugaudit-mcp                    Run the server for your AI agent
";

/// The help text, with ANSI color applied only when stdout is a real
/// terminal. Mirrors the `USAGE` const byte-for-byte when stdout is piped
/// or captured (CI, files, the test harness), so redirected help never
/// contains escape sequences. On a terminal, the tagline is bold and the
/// four section headings (`USAGE:`, `COMMANDS:`, …) are highlighted so
/// the reference reads faster; the command/example bodies stay plain.
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

/// Errors from `connect`. Split out from `CliError` because connect has its
/// own failure surface (agent CLI missing, registration command failed)
/// that benefits from distinct, actionable messages.
#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("invalid agent choice; choose one of the listed options")]
    InvalidChoice,
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

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
