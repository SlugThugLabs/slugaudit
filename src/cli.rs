//! The `slugaudit` binary's command-line surface: `serve` (the MCP server,
//! the default with no arguments). `connect` registers the installed
//! SlugAudit binary — or the running binary when no installed copy exists —
//! as a stdio MCP server named `slugaudit` in an AI agent or editor.
//! `disconnect` removes it. `install` copies the binary to a stable path.
//!
//! CLI output goes through [`out`] and [`err`] rather than `print!`:
//! a write to a closed pipe must not panic the binary.

mod usage;

use thiserror::Error;

pub use usage::usage;

// Read by `cli_tests` through `use super::*`; nothing in a non-test build
// reads the unstyled text or the fixed-style renderer.
#[cfg(test)]
use crate::util::Style;
#[cfg(test)]
pub(crate) use usage::USAGE;
#[cfg(test)]
use usage::render_usage;

/// The subcommand selected by the user from CLI arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Start the MCP server over stdio.
    Serve,
    /// Connect SlugAudit to an AI agent or editor.
    Connect {
        /// Agent to connect directly; `None` offers the detected agents.
        agent: Option<String>,
    },
    /// Disconnect SlugAudit from an AI agent or editor.
    Disconnect {
        /// Agent to disconnect directly; `None` offers the connected agents.
        agent: Option<String>,
    },
    /// Install the binary to the standard local binary path.
    Install,
    /// Open the interactive setup menu.
    Menu,
    /// Check for and apply self-updates.
    Update,
    /// Print version information.
    Version,
    /// Print help and usage instructions.
    Help,
}

/// Parses command-line arguments into a high-level [`Command`].
///
/// Returns `Ok(Command::Serve)` if no arguments were provided.
#[must_use = "the returned Command must be dispatched by the caller"]
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Ok(Command::Serve);
    };
    match first.as_str() {
        "serve" => Ok(Command::Serve),
        "help" => Ok(Command::Help),
        "connect" => Ok(Command::Connect {
            agent: single_agent("connect", args)?,
        }),
        "disconnect" | "remove" => Ok(Command::Disconnect {
            agent: single_agent(first.as_str(), args)?,
        }),
        "install" => Ok(Command::Install),
        "menu" => Ok(Command::Menu),
        "update" => Ok(Command::Update),
        "version" | "--version" | "-V" | "-v" => Ok(Command::Version),
        "--help" | "-h" => Ok(Command::Help),
        other => Err(format!(
            "unknown command {other:?}; expected one of: serve, connect, disconnect, install, update, menu, version, help"
        )),
    }
}

/// Takes the optional agent name for `connect`/`disconnect`; a second
/// argument is a usage error rather than being silently ignored.
fn single_agent(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<Option<String>, String> {
    let agent = args.next();
    match args.next() {
        Some(extra) => Err(format!(
            "`{command}` takes at most one agent name; unexpected argument {extra:?}"
        )),
        None => Ok(agent),
    }
}

/// Writes `args` to `writer` and flushes it. Private so tests can drive the
/// failure paths with an injected writer; [`out`] and [`err`] are the process
/// wrappers the CLI actually calls.
pub(crate) fn write_to(
    writer: &mut impl std::io::Write,
    args: std::fmt::Arguments<'_>,
) -> std::io::Result<()> {
    writer.write_fmt(args)?;
    writer.flush()
}

/// Writes formatted text to stdout, returning a closed pipe or any other
/// write failure instead of panicking.
///
/// # Errors
///
/// Returns the underlying I/O error; `BrokenPipe` means the reader went away.
pub fn out(args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    write_to(&mut std::io::stdout().lock(), args)
}

/// Writes formatted text to stderr, returning the write failure instead of
/// panicking.
///
/// # Errors
///
/// Returns the underlying I/O error.
pub fn err(args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    write_to(&mut std::io::stderr().lock(), args)
}

/// Whether `error` or a cause in its chain is a closed pipe (`EPIPE`). The
/// CLI treats that as a clean exit: the reader went away, not the command.
#[must_use]
pub fn is_broken_pipe(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
        {
            return true;
        }
        current = error.source();
    }
    false
}

/// Errors encountered while connecting or disconnecting an agent client.
#[derive(Debug, Error)]
pub enum ConnectError {
    /// The user picked an option outside the presented list.
    #[error("invalid agent choice; choose one of the listed options")]
    InvalidChoice,
    /// Standard input closed before the prompt was answered.
    #[error("input ended before an answer was given")]
    EndOfInput,
    /// The running binary's path could not be resolved.
    #[error("binary path unavailable: {0}")]
    BinaryPath(std::io::Error),
    /// Neither a home directory nor a usable config parent was available.
    #[error("home directory is unavailable")]
    HomeUnavailable,
    /// No registry entry matches the requested agent name.
    #[error("unknown agent {0:?}; run `slugaudit connect` to see detected agents")]
    UnknownAgent(String),
    /// The client's executable could not be spawned or waited on.
    #[error("failed to run `{cli}`: {inner}")]
    AgentCommand {
        /// Client executable that could not be run.
        cli: String,
        /// Underlying spawn or wait error.
        inner: std::io::Error,
    },
    /// The client's `mcp remove` subcommand exited non-zero.
    #[error("`{cli} mcp remove` exited with {status}")]
    RemoveFailed {
        /// Client executable whose remove failed.
        cli: String,
        /// Exit status as printed by the OS.
        status: String,
    },
    /// The client's `mcp add` subcommand exited non-zero after printing output.
    #[error("`{cli} mcp add` exited with {status} — see the output above")]
    AddFailed {
        /// Client executable whose add failed.
        cli: String,
        /// Exit status as printed by the OS.
        status: String,
    },
    /// The client rejected `mcp add` and printed nothing, so there is no
    /// "output above" to point the user at.
    #[error("`{cli} mcp add` exited with {status} and printed no output")]
    AddFailedNoOutput {
        /// Client executable whose add failed.
        cli: String,
        /// Exit status as printed by the OS.
        status: String,
    },
    /// The client has no automated registration path; carries the reason the
    /// caller reports.
    #[error("{agent} cannot be connected automatically: {reason}")]
    UnsupportedAgent {
        /// Human-readable client name.
        agent: String,
        /// Why the automated path is refused.
        reason: String,
    },
    /// The config's expected section exists but is not a JSON object.
    #[error(
        "cannot connect: \"{section}\" in the configuration is not an object, so there is \
         nowhere to write the slugaudit entry. Fix or remove that key, then retry"
    )]
    SectionNotAnObject {
        /// Top-level section key that is not an object.
        section: String,
    },
    /// The config file's top-level JSON value is not an object.
    #[error(
        "cannot connect: {path} does not hold a JSON object at its top level, so there is \
         nowhere to write the slugaudit entry. Fix or remove the file, then retry"
    )]
    RootNotAnObject {
        /// Config file that could not be written.
        path: std::path::PathBuf,
    },
    /// An existing config file could not be parsed as JSON.
    #[error(
        "cannot modify {path}: configuration file exists but contains invalid JSON: {source} (fix syntax/comments or remove the file to reset, then retry)"
    )]
    InvalidConfig {
        /// Config file that failed to parse.
        path: std::path::PathBuf,
        /// Parser error, including line and column.
        #[source]
        source: serde_json::Error,
    },
    /// A JSON value could not be encoded or decoded.
    #[error("JSON config error: {0}")]
    Json(#[from] serde_json::Error),
    // Spelled out rather than `transparent`: transparent forwards `source()`
    // through the io::Error, which hides the error itself from
    // `is_broken_pipe`'s chain walk.
    /// An I/O operation failed. A closed pipe is treated as a clean exit.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
