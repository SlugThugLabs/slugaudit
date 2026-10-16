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
    match cli::parse_args(args) {
        Ok(cmd) => match cmd {
            Command::Connect { agent } => {
                if let Err(error) = connect::run_connect(agent.as_deref()) {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            Command::Disconnect { agent } => {
                if let Err(error) = connect::run_disconnect(agent.as_deref()) {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            Command::Install => {
                if let Err(error) = install::run_install() {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            Command::Update => {
                if let Err(error) = update::run_update() {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            Command::Help => {
                print!("{}", cli::usage());
                return Ok(());
            }
            Command::Version => {
                println!("slugaudit {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Command::Menu => {
                if !menu::run_menu()? {
                    return Ok(());
                }
            }
            Command::Serve => {}
        },
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
    }

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
    SlugAuditServer::new()
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
