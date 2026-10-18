#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]

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
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    };
    // `std::process::exit` is deliberate here and confined to `main` (the
    // one place the denied `clippy::exit` lint permits it): the four
    // side-effecting CLI commands report `Error: {reason}` and exit 1.
    match run_command(command) {
        CommandOutcome::Serve => {}
        CommandOutcome::Done => return Ok(()),
        CommandOutcome::Reported(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
        CommandOutcome::Propagated(error) => return Err(error),
    }
    init_logging();
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
    /// `Error: {0}` and exit 1.
    Reported(String),
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
        Command::Help => {
            print!("{}", cli::usage());
            CommandOutcome::Done
        }
        Command::Version => {
            println!("slugaudit {}", env!("CARGO_PKG_VERSION"));
            CommandOutcome::Done
        }
        Command::Menu => match menu::run_menu() {
            Ok(true) => CommandOutcome::Serve,
            Ok(false) => CommandOutcome::Done,
            Err(error) => CommandOutcome::Propagated(error),
        },
        Command::Serve => CommandOutcome::Serve,
    }
}

/// Maps a side-effecting command's `Result` onto its exit contract.
fn reported(result: Result<(), impl std::fmt::Display>) -> CommandOutcome {
    match result {
        Ok(()) => CommandOutcome::Done,
        Err(error) => CommandOutcome::Reported(error.to_string()),
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

/// Resolves `RUST_LOG` and `SLUGAUDIT_LOG_FORMAT`. An unset variable takes its
/// default silently; a set one that is invalid or not UTF-8 falls back to the
/// default (`info`, plain text) and is recorded in `rejected` rather than
/// dropped, so the operator learns why the setting did not apply.
fn log_settings(
    rust_log: Result<String, std::env::VarError>,
    format: Result<String, std::env::VarError>,
) -> LogSettings {
    let mut rejected = Vec::new();
    let filter = match rust_log {
        Ok(directives) => EnvFilter::try_new(&directives).unwrap_or_else(|error| {
            rejected.push(format!(
                "RUST_LOG was rejected and the filter is `info`: {error}"
            ));
            EnvFilter::new("info")
        }),
        Err(std::env::VarError::NotPresent) => EnvFilter::new("info"),
        Err(error @ std::env::VarError::NotUnicode(_)) => {
            rejected.push(format!(
                "RUST_LOG was rejected and the filter is `info`: {error}"
            ));
            EnvFilter::new("info")
        }
    };
    let json = match format {
        Ok(value) => value.eq_ignore_ascii_case("json"),
        Err(std::env::VarError::NotPresent) => false,
        Err(error @ std::env::VarError::NotUnicode(_)) => {
            rejected.push(format!(
                "SLUGAUDIT_LOG_FORMAT was rejected and logs stay plain text: {error}"
            ));
            false
        }
    };
    LogSettings {
        filter,
        json,
        rejected,
    }
}

/// Installs the stderr tracing subscriber for `serve`. Logs go to stderr
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
mod tests {
    use super::*;

    #[test]
    fn unset_log_variables_take_their_defaults_without_a_complaint() {
        let settings = log_settings(
            Err(std::env::VarError::NotPresent),
            Err(std::env::VarError::NotPresent),
        );

        assert!(!settings.json);
        assert!(settings.rejected.is_empty());
    }

    #[test]
    fn an_invalid_rust_log_is_reported_not_dropped() {
        let settings = log_settings(
            Ok("slugaudit=trace,,=bogus[".to_owned()),
            Err(std::env::VarError::NotPresent),
        );

        assert_eq!(settings.rejected.len(), 1, "{:?}", settings.rejected);
        assert!(settings.rejected[0].contains("RUST_LOG"));
    }

    #[test]
    fn a_valid_rust_log_and_json_format_are_honored_silently() {
        let settings = log_settings(Ok("debug".to_owned()), Ok("JSON".to_owned()));

        assert!(settings.json);
        assert!(settings.rejected.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_log_format_is_reported_not_treated_as_plain() {
        use std::os::unix::ffi::OsStringExt as _;
        let bad = std::ffi::OsString::from_vec(vec![0xff]);

        let settings = log_settings(
            Err(std::env::VarError::NotPresent),
            Err(std::env::VarError::NotUnicode(bad)),
        );

        assert!(!settings.json);
        assert_eq!(settings.rejected.len(), 1);
        assert!(settings.rejected[0].contains("SLUGAUDIT_LOG_FORMAT"));
    }
}
