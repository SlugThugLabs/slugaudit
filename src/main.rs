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

/// Installs the stderr tracing subscriber for `serve`. Logs go to stderr
/// only — stdout is the JSON-RPC transport. `SLUGAUDIT_LOG_FORMAT=json`
/// switches the formatter to JSON; anything else stays the plain writer
/// with ANSI off (stderr may be a captured file, not a terminal).
fn init_logging() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let is_json = std::env::var("SLUGAUDIT_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if is_json {
        tracing_subscriber::fmt()
            .json()
            .with_writer(std::io::stderr)
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .with_env_filter(env_filter)
            .init();
    }
}
