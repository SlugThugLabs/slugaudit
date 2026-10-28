//! The `slugaudit` binary: dispatch one CLI command, or serve the MCP server
//! over stdio when the command is `serve` (the default with no arguments).
//!
//! Arguments are parsed before logging is installed, so a parse error is
//! reported directly and exits 1. Logs go to stderr only — stdout is the
//! JSON-RPC transport — and `SLUGAUDIT_LOG_FORMAT=json` selects a JSON
//! formatter.

#![forbid(unsafe_code)]

use rmcp::{ServiceExt, transport::stdio};
use slugaudit_mcp_rust::cli::{self, Command};
use slugaudit_mcp_rust::connect;
use slugaudit_mcp_rust::install;
use slugaudit_mcp_rust::menu;
use slugaudit_mcp_rust::server::SlugAuditServer;
use slugaudit_mcp_rust::update;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `std::env::args()` panics on any argument that is not valid Unicode,
    // and on Linux argv is bytes: a path typed or completed from a Latin-1
    // filename reaches here unmodified. Reproduced against this binary -
    // `slugaudit $'\xff'` exited 101 with `panicked at library/std/src/env.rs:
    // called Result::unwrap() on an Err value` - so the lossy conversion is
    // what keeps a bad argument a rejected command rather than a crash.
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    let command = match cli::parse_args(args) {
        Ok(cmd) => cmd,
        Err(error) => {
            cli::err(format_args!("Error: {error}\n"))?;
            std::process::exit(1);
        }
    };
    // The subscriber is installed before dispatch: CLI commands log too
    // (probe/install/update warnings), and a warning raised before this ran
    // reached no subscriber and was dropped. Serve mode keeps its subscriber
    // because the same initialization serves both paths; installing it twice
    // would panic, so this is the only call.
    init_logging();
    // `std::process::exit` is deliberate here and confined to `main` (the
    // one place the denied `clippy::exit` lint permits it): the four
    // side-effecting CLI commands report `Error: {reason}` and exit 1.
    match run_command(command) {
        CommandOutcome::Serve => {}
        CommandOutcome::Done => return Ok(()),
        CommandOutcome::Reported(error) if cli::is_broken_pipe(error.as_ref()) => return Ok(()),
        CommandOutcome::Reported(error) => {
            cli::err(format_args!("Error: {error}\n"))?;
            std::process::exit(1);
        }
        CommandOutcome::Propagated(error) if cli::is_broken_pipe(error.as_ref()) => return Ok(()),
        CommandOutcome::Propagated(error) => return Err(error),
    }
    SlugAuditServer::new()
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

/// What `main` should do after running one CLI command.
enum CommandOutcome {
    /// Fall through to serving the MCP server on stdio (`serve`, and
    /// `menu` when the user chose to start it).
    Serve,
    /// The command finished; exit 0.
    Done,
    /// The command failed with a human-reportable error: print
    /// `Error: {0}` and exit 1, unless the cause is a closed stdout.
    Reported(Box<dyn std::error::Error>),
    /// The `menu` path failed with a boxed error; propagate out of `main`
    /// exactly as the old `run_menu()?` did.
    Propagated(Box<dyn std::error::Error>),
}

/// Runs one parsed command and reports how `main` should terminate. The
/// arms are a decision table — no side effects beyond the command's own.
fn run_command(command: Command) -> CommandOutcome {
    match command {
        Command::Connect { agent } => reported(connect::run_connect(agent.as_deref())),
        Command::Disconnect { agent } => reported(connect::run_disconnect(agent.as_deref())),
        Command::Install => reported(install::run_install()),
        Command::Update => reported(update::run_update()),
        Command::Help => reported(cli::out(format_args!("{}", cli::usage()))),
        Command::Version => reported(cli::out(format_args!(
            "slugaudit {}\n",
            env!("CARGO_PKG_VERSION")
        ))),
        Command::Menu => match menu::run_menu() {
            Ok(true) => CommandOutcome::Serve,
            Ok(false) => CommandOutcome::Done,
            Err(error) => CommandOutcome::Propagated(error),
        },
        Command::Serve => CommandOutcome::Serve,
    }
}

/// Maps a side-effecting command's `Result` onto its exit contract.
fn reported(result: Result<(), impl std::error::Error + 'static>) -> CommandOutcome {
    match result {
        Ok(()) => CommandOutcome::Done,
        Err(error) => CommandOutcome::Reported(Box::new(error)),
    }
}

/// Logging choices read from the environment, plus every setting the
/// environment carried that could not be honored.
struct LogSettings {
    filter: EnvFilter,
    json: bool,
    /// One message per rejected setting. They are logged once the subscriber
    /// is installed, because nothing can be logged before it.
    rejected: Vec<String>,
}

/// The filter used when `RUST_LOG` is unset or rejected: dependencies are
/// capped at warn, this crate keeps its per-call info lines, and rmcp's
/// `response error` warn (its `Debug` dump of an `ErrorData`) is capped at
/// error because `server_runner::log_completion` already logs each tool
/// failure once.
const DEFAULT_LOG_FILTER: &str = "warn,slugaudit_mcp_rust=info,rmcp=error";

/// Resolves `RUST_LOG` and `SLUGAUDIT_LOG_FORMAT`. An unset variable takes its
/// default silently; a set one that is invalid or not UTF-8 falls back to the
/// default ([`DEFAULT_LOG_FILTER`], plain text) and is recorded in `rejected` rather than
/// dropped, so the operator learns why the setting did not apply.
fn log_settings(
    rust_log: Result<String, std::env::VarError>,
    format: Result<String, std::env::VarError>,
) -> LogSettings {
    let mut rejected = Vec::new();
    let filter = match rust_log {
        Ok(directives) => EnvFilter::try_new(&directives).unwrap_or_else(|error| {
            rejected.push(format!(
                "RUST_LOG was rejected and the filter is `{DEFAULT_LOG_FILTER}`: {error}"
            ));
            EnvFilter::new(DEFAULT_LOG_FILTER)
        }),
        Err(std::env::VarError::NotPresent) => EnvFilter::new(DEFAULT_LOG_FILTER),
        // `VarError`'s Display quotes the raw environment value, so the
        // rejection is reported as a static message; the value itself is
        // never logged.
        Err(std::env::VarError::NotUnicode(_)) => {
            rejected.push("RUST_LOG is not valid Unicode; ignoring it".to_owned());
            EnvFilter::new(DEFAULT_LOG_FILTER)
        }
    };
    let json = match format {
        Ok(value) => value.eq_ignore_ascii_case("json"),
        Err(std::env::VarError::NotPresent) => false,
        // As above: never interpolate the value-carrying `VarError` Display.
        Err(std::env::VarError::NotUnicode(_)) => {
            rejected.push("SLUGAUDIT_LOG_FORMAT is not valid Unicode; using text".to_owned());
            false
        }
    };
    LogSettings {
        filter,
        json,
        rejected,
    }
}

/// Installs the process-wide stderr tracing subscriber, before any command is
/// dispatched, so CLI warnings are not dropped. Logs go to stderr
/// only — stdout is the JSON-RPC transport. `SLUGAUDIT_LOG_FORMAT=json`
/// switches the formatter to JSON; anything else stays the plain writer
/// with ANSI off (stderr may be a captured file, not a terminal).
fn init_logging() {
    let settings = log_settings(
        std::env::var("RUST_LOG"),
        std::env::var("SLUGAUDIT_LOG_FORMAT"),
    );
    if settings.json {
        tracing_subscriber::fmt()
            .json()
            .with_writer(std::io::stderr)
            .with_env_filter(settings.filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .with_env_filter(settings.filter)
            .init();
    }
    for message in &settings.rejected {
        tracing::warn!("{message}");
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
