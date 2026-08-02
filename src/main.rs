#![forbid(unsafe_code)]

use rmcp::{ServiceExt, transport::stdio};
use slugaudit_mcp_rust::server::SlugAuditServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    SlugAuditServer::new()
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
