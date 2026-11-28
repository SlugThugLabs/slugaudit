//! The `slugaudit` binary's command-line surface: `serve` (the MCP server,
//! the default with no arguments). `connect` registers the installed
//! SlugAudit binary — or the running binary when no installed copy exists —
//! as a stdio MCP server named `slugaudit` in an AI agent or editor.
//! `disconnect` removes it. `install` copies the binary to a stable path.
//!
//! CLI output goes through [`out`] and [`err`] rather than `print!`:
//! a write to a closed pipe must not panic the binary.

mod usage;

pub use crate::connect::ConnectError;
pub use usage::{Topic, topic_usage, usage};

// Read by `cli_tests` through `use super::*`; nothing in a non-test build
// reads the unstyled text or the fixed-style renderer.
#[cfg(test)]
use crate::util::Style;
#[cfg(test)]
use usage::render_usage;
#[cfg(test)]
pub(crate) use usage::usage_text;

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
    /// Inspect or install optional non-Rust Tree-sitter grammars.
    Grammar(crate::grammar_cli::GrammarAction),
    /// Open the interactive setup menu.
    Menu,
    /// Check for and apply self-updates.
    Update,
    /// Print version information.
    Version,
    /// Print help and usage instructions.
    Help,
    /// Print one subcommand's usage; requested by `--help`/`-h` anywhere in
    /// that subcommand's arguments. Never runs the subcommand.
    CommandHelp(Topic),
}

/// Parses command-line arguments into a high-level [`Command`].
///
/// Returns `Ok(Command::Serve)` if no arguments were provided.
///
/// # Errors
///
/// Returns a usage message when the command is unknown, or when a command
/// receives arguments it does not accept.
#[must_use = "the returned Command must be dispatched by the caller"]
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Ok(Command::Serve);
    };
    // Help is decided before any per-command argument handling so that no
    // position of `--help`/`-h` can reach a command with side effects.
    let rest: Vec<String> = args.collect();
    if let Some(topic) = Topic::for_command(first.as_str())
        && rest
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok(Command::CommandHelp(topic));
    }
    if matches!(first.as_str(), "--version" | "-V" | "-v")
        && rest
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok(Command::CommandHelp(Topic::Version));
    }
    if matches!(first.as_str(), "--help" | "-h")
        && rest
            .iter()
            .all(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok(Command::Help);
    }
    let args = rest.into_iter();
    match first.as_str() {
        "serve" => no_arguments("serve", args).map(|()| Command::Serve),
        "help" | "--help" | "-h" => no_arguments("help", args).map(|()| Command::Help),
        "connect" => Ok(Command::Connect {
            agent: single_agent("connect", args)?,
        }),
        "disconnect" | "remove" => Ok(Command::Disconnect {
            agent: single_agent(first.as_str(), args)?,
        }),
        "install" => no_arguments("install", args).map(|()| Command::Install),
        "grammar" => parse_grammar_args(args).map(Command::Grammar),
        "menu" => no_arguments("menu", args).map(|()| Command::Menu),
        "update" => no_arguments("update", args).map(|()| Command::Update),
        "version" | "--version" | "-V" | "-v" => {
            no_arguments("version", args).map(|()| Command::Version)
        }
        other => Err(format!(
            "unknown command {other:?}; expected one of: serve, connect, disconnect, install, grammar, update, menu, version, help"
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

/// Rejects any argument after a command that takes none; a stray argument is
/// a usage error rather than being silently ignored.
fn no_arguments(command: &str, mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next() {
        Some(extra) => Err(format!(
            "`{command}` takes no arguments; unexpected argument {extra:?}"
        )),
        None => Ok(()),
    }
}

fn parse_grammar_args(
    mut args: impl Iterator<Item = String>,
) -> Result<crate::grammar_cli::GrammarAction, String> {
    let Some(sub) = args.next() else {
        return Err("`grammar` requires a subcommand: `list` or `install <LANGUAGE>`".to_owned());
    };
    match sub.as_str() {
        "list" => {
            no_arguments("grammar list", args)?;
            Ok(crate::grammar_cli::GrammarAction::List)
        }
        "install" => {
            let mut language: Option<String> = None;
            let mut yes = false;
            for arg in args {
                match arg.as_str() {
                    "--yes" | "-y" => yes = true,
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag {other:?} for `grammar install`"));
                    }
                    _ if language.is_none() => language = Some(arg),
                    extra => {
                        return Err(format!(
                            "`grammar install` takes one language; unexpected argument {extra:?}"
                        ));
                    }
                }
            }
            let language = language.ok_or_else(|| {
                "`grammar install` requires a language name (e.g. `slugaudit grammar install python`)"
                    .to_owned()
            })?;
            Ok(crate::grammar_cli::GrammarAction::Install { language, yes })
        }
        other => Err(format!(
            "unknown `grammar` subcommand {other:?}; expected `list` or `install`"
        )),
    }
}

/// Writes `args` to `writer` and flushes it. Private so tests can drive the
/// failure paths with an injected writer; [`out`] and [`err`] are the process
/// wrappers the CLI actually calls.
pub(crate) fn write_to(
    writer: &mut (impl std::io::Write + ?Sized),
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

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
