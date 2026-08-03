//! The `slugaudit-mcp-rust` binary's command-line surface: `serve` (the
//! MCP server, the default with no arguments) and `enable`/`disable` — the
//! one human-facing control described in `ARCHITECTURE.md`, exposed here
//! as real commands instead of a "go create this directory yourself"
//! workaround. `enable` also runs the project's first import immediately,
//! before returning, rather than waiting for an AI to make the first tool
//! call.
use crate::project::{self, ProjectRoot};
use crate::{parse, store, sync};
use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Enable { path: PathBuf },
    Disable { path: PathBuf, assume_yes: bool },
    Help,
}

#[must_use]
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Command {
    let Some(first) = args.next() else {
        return Command::Serve;
    };
    match first.as_str() {
        "serve" => Command::Serve,
        "enable" => Command::Enable {
            path: args
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from),
        },
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
            Command::Disable {
                path: path.unwrap_or_else(|| PathBuf::from(".")),
                assume_yes,
            }
        }
        _ => Command::Help,
    }
}

pub const USAGE: &str = "\
slugaudit-mcp-rust — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit-mcp-rust                    Run the MCP server (stdio transport)
    slugaudit-mcp-rust serve              Same as running with no arguments
    slugaudit-mcp-rust enable [PATH]      Turn SlugAudit on for PATH (default: .)
                                           and run its first import immediately
    slugaudit-mcp-rust disable [PATH]     Turn SlugAudit off for PATH (default: .),
                                           deleting its database, findings, and evidence
        -y, --yes                         Skip the confirmation prompt
    slugaudit-mcp-rust help               Show this message
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
