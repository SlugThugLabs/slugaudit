//! The `slugaudit-mcp-rust` binary's command-line surface: `serve` (the
//! MCP server, the default with no arguments). `connect` registers the
//! running binary as a stdio MCP server named `slugaudit` in a supported AI
//! agent (Claude Code, Grok, or Codex). `install` copies the binary to a
//! stable path so it survives rebuilds.
#![allow(clippy::print_stdout)]

use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
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

    pub(crate) fn cli_name(self) -> &'static str {
        match self {
            ConnectAgent::Claude => "claude",
            ConnectAgent::Grok => "grok",
            ConnectAgent::Codex => "codex",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
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
        other => Err(format!(
            "unknown command {other:?}; expected one of: serve, connect, install, help"
        )),
    }
}

pub const USAGE: &str = "\
slugaudit-mcp — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit-mcp                    Run the MCP server (stdio transport)
    slugaudit-mcp serve              Same as running with no arguments
    slugaudit-mcp connect [AGENT]    Register this binary as the `slugaudit`
                                      MCP server in an AI agent. AGENT is one
                                      of: claude, grok, codex. Omit to pick
                                      from an interactive menu.
    slugaudit-mcp install            Copy this binary to ~/.slugthug/bin/ so
                                      it's on a stable path shared with future
                                      slug-branded products.
    slugaudit-mcp help               Show this message
";

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
