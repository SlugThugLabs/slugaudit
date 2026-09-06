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
    match cli::parse_args(std::env::args().skip(1)) {
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
