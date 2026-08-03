#![forbid(unsafe_code)]

use rmcp::{ServiceExt, transport::stdio};
use slugaudit_mcp_rust::server::SlugAuditServer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
