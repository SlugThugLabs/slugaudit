//! The `slugaudit` binary: dispatch one CLI command, or serve the MCP server
//! over stdio when the command is `serve` (the default with no arguments).
//!
//! Arguments are parsed before logging is installed, so a parse error is
//! reported directly and exits 2 (usage). Runtime failures exit 1. Logs go to stderr only — stdout is the
//! JSON-RPC transport — and `SLUGAUDIT_LOG_FORMAT=json` selects a JSON
//! formatter.

#![forbid(unsafe_code)]

use rmcp::{ServiceExt, service::ServerInitializeError, transport::stdio};
use slugaudit_mcp_rust::cli::{self, Command};
use slugaudit_mcp_rust::connect;
use slugaudit_mcp_rust::install;
use slugaudit_mcp_rust::menu;
use slugaudit_mcp_rust::model::{EnvConfigError, init_process_limits};
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
            std::process::exit(2);
        }
    };
    // The subscriber is installed before dispatch: CLI commands log too
    // (probe/install/update warnings), and a warning raised before this ran
    // reached no subscriber and was dropped. Serve mode keeps its subscriber
    // because the same initialization serves both paths; installing it twice
    // would panic, so this is the only call.
    if let Err(error) = init_logging() {
        cli::err(format_args!("Error: {error}\n"))?;
        std::process::exit(1);
    }
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
        CommandOutcome::Propagated(error) => {
            cli::err(format_args!("Error: {error}\n"))?;
            std::process::exit(1);
        }
    }
    // A set-but-invalid SLUGAUDIT_* variable, or an Adaptive profile on a host
    // whose memory cannot be detected, stops the server here rather than
    // running with limits the operator did not ask for.
    if let Err(error) = init_process_limits() {
        cli::err(format_args!("Error: {error}\n"))?;
        std::process::exit(1);
    }
    match serve_until_done().await {
        Ok(ServeEnd::Finished) => Ok(()),
        Ok(ServeEnd::InputClosedEarly) => {
            cli::err(format_args!(
                "slugaudit: input closed before initialize; exiting\n"
            ))?;
            Ok(())
        }
        Ok(ServeEnd::Signalled(name)) => {
            cli::err(format_args!("slugaudit: received {name}; shutting down\n"))?;
            // Returning would make the runtime drop wait on the blocking
            // stdin reader until the host closes stdin; stderr is unbuffered,
            // so exit now with status 0.
            std::process::exit(0);
        }
        Err(error) => {
            cli::err(format_args!("Error: {error}\n"))?;
            std::process::exit(1);
        }
    }
}

/// How serving the MCP session ended without a failure.
enum ServeEnd {
    /// The host closed stdin after the session ran.
    Finished,
    /// Stdin closed before the client sent `initialize`.
    InputClosedEarly,
    /// SIGTERM or SIGINT (Ctrl-C) asked the process to stop.
    Signalled(&'static str),
}

/// Serves MCP over stdio until the session ends or a termination signal
/// arrives. Dropping the session future on a signal cancels the running
/// service, so in-flight work is drained by the runtime shutdown.
async fn serve_until_done() -> Result<ServeEnd, Box<dyn std::error::Error>> {
    let session = async {
        match SlugAuditServer::new().serve(stdio()).await {
            Ok(running) => {
                running.waiting().await?;
                Ok(ServeEnd::Finished)
            }
            Err(ServerInitializeError::ConnectionClosed(_)) => Ok(ServeEnd::InputClosedEarly),
            Err(error) => Err(error.into()),
        }
    };
    tokio::select! {
        outcome = session => outcome,
        signal = termination_signal() => Ok(ServeEnd::Signalled(signal?)),
    }
}

/// Resolves with the name of the first termination signal received.
#[cfg(unix)]
async fn termination_signal() -> std::io::Result<&'static str> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = terminate.recv() => Ok("SIGTERM"),
        interrupt = tokio::signal::ctrl_c() => interrupt.map(|()| "SIGINT"),
    }
}

/// Resolves when Ctrl-C is received (the only portable signal).
#[cfg(not(unix))]
async fn termination_signal() -> std::io::Result<&'static str> {
    tokio::signal::ctrl_c().await.map(|()| "Ctrl-C")
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

/// Logging choices read from the environment.
struct LogSettings {
    filter: EnvFilter,
    json: bool,
}

/// The filter used when `RUST_LOG` is unset: dependencies are
/// capped at warn, this crate keeps its per-call info lines, and rmcp's
/// `response error` warn (its `Debug` dump of an `ErrorData`) is capped at
/// error because `server_runner::log_completion` already logs each tool
/// failure once.
const DEFAULT_LOG_FILTER: &str = "warn,slugaudit_mcp_rust=info,rmcp=error";

/// What a valid `RUST_LOG` is, for the startup error.
const RUST_LOG_EXPECTED: &str =
    "tracing filter directives such as `info` or `slugaudit_mcp_rust=debug`";

/// What a valid `SLUGAUDIT_LOG_FORMAT` is, for the startup error.
const LOG_FORMAT_EXPECTED: &str = "`json` or `text`";

/// Resolves `RUST_LOG` and `SLUGAUDIT_LOG_FORMAT`. An unset variable takes its
/// default ([`DEFAULT_LOG_FILTER`], plain text) silently. A set `RUST_LOG`
/// that is invalid or not UTF-8 is an error naming the variable (never its
/// value, which the filter parser's message may quote), so the server does
/// not run with a different filter than the operator asked for. A non-UTF-8
/// `SLUGAUDIT_LOG_FORMAT` is the same kind of error.
fn log_settings(
    rust_log: Result<String, std::env::VarError>,
    format: Result<String, std::env::VarError>,
) -> Result<LogSettings, EnvConfigError> {
    let filter = match rust_log {
        Ok(directives) => EnvFilter::try_new(&directives).map_err(|_| EnvConfigError::Invalid {
            variable: "RUST_LOG".to_owned(),
            expected: RUST_LOG_EXPECTED.to_owned(),
        })?,
        Err(std::env::VarError::NotPresent) => EnvFilter::new(DEFAULT_LOG_FILTER),
        // `VarError`'s Display quotes the raw environment value, so only the
        // variable name is reported.
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(EnvConfigError::NotUnicode {
                variable: "RUST_LOG".to_owned(),
                expected: RUST_LOG_EXPECTED.to_owned(),
            });
        }
    };
    let json = match format {
        Ok(value) => value.eq_ignore_ascii_case("json"),
        Err(std::env::VarError::NotPresent) => false,
        // As above: never interpolate the value-carrying `VarError` Display.
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(EnvConfigError::NotUnicode {
                variable: "SLUGAUDIT_LOG_FORMAT".to_owned(),
                expected: LOG_FORMAT_EXPECTED.to_owned(),
            });
        }
    };
    Ok(LogSettings { filter, json })
}

/// Installs the process-wide stderr tracing subscriber, before any command is
/// dispatched, so CLI warnings are not dropped. Logs go to stderr
/// only — stdout is the JSON-RPC transport. `SLUGAUDIT_LOG_FORMAT=json`
/// switches the formatter to JSON; anything else stays the plain writer
/// with ANSI off (stderr may be a captured file, not a terminal).
///
/// # Errors
///
/// Returns [`EnvConfigError`] for a set-but-invalid `RUST_LOG` or a non-UTF-8
/// `SLUGAUDIT_LOG_FORMAT`.
fn init_logging() -> Result<(), EnvConfigError> {
    let settings = log_settings(
        std::env::var("RUST_LOG"),
        std::env::var("SLUGAUDIT_LOG_FORMAT"),
    )?;
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
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
