#![forbid(unsafe_code)]

use rmcp::{ServiceExt, transport::stdio};
use slugaudit_mcp_rust::cli::{self, Command};
use slugaudit_mcp_rust::connect;
use slugaudit_mcp_rust::install;
use slugaudit_mcp_rust::menu;
use slugaudit_mcp_rust::server::SlugAuditServer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match cli::parse_args(std::env::args().skip(1)) {
        Ok(cmd) => match cmd {
            Command::Connect { agent } => {
                match agent {
                    Some(agent) => connect::run_connect(agent)?,
                    None => connect::run_connect_interactive()?,
                }
                return Ok(());
            }
            Command::Install => return Ok(install::run_install()?),
            Command::Help => {
                print!("{}", cli::USAGE);
                return Ok(());
            }
            Command::Version => {
                // `CARGO_PKG_VERSION` is compile-time (0.1.0); the binary
                // prints it so a user can verify a download against its
                // release tag and checksum.
                println!("slugaudit-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Command::Menu => {
                // The menu drives install/connect itself; only when the
                // user picks "run the server now" does it return `true`
                // and fall through to serve below.
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

    // Diagnostics only ever go to stderr — stdout is the MCP transport and
    // must stay strictly JSON-RPC. `RUST_LOG` overrides the default level;
    // unset, tool call spans/events log at `info` without drowning in
    // per-row/per-poll `debug` noise from dependencies. ANSI color is
    // disabled unconditionally: the MCP host that spawns this process
    // pipes stderr for its own logging, not a terminal, and color escape
    // codes would corrupt any log aggregation/parsing on the other end.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    SlugAuditServer::new()
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
