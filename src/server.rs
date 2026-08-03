use crate::tools;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use std::sync::Arc;
use tokio::sync::Semaphore;

const INSTRUCTIONS: &str = "SlugAudit supplies searchable, trustworthy evidence about a codebase — \
    parsed structure, symbols, imports, diagnostics, and prior AI-reviewed findings. Use `report` \
    for an automatic snapshot of what evidence exists, `query` for arbitrary read-only SQL against \
    the project's own database (search, symbol/import/diagnostic lookup, and source retrieval all \
    reach through it), `structure` for Tree-sitter structural pattern matching, and `finding` to \
    persist a conclusion you have actually reviewed. Dependency graph resolution and background \
    activation are reserved for future versions. SlugAudit performs no automated risk detection \
    and reaches no conclusions itself: it supplies evidence, the calling AI performs all judgment.";

/// Every tool handler does filesystem discovery, SQLite I/O, and (on first
/// sync) Tree-sitter parsing — all synchronous, potentially slow work that
/// would otherwise run directly on a Tokio worker thread and starve other
/// tasks scheduled on it. Bounds how many such blocking operations run at
/// once, independent of how many concurrent tool calls arrive.
const MAX_CONCURRENT_BLOCKING_OPS: usize = 8;

#[derive(Clone)]
pub struct SlugAuditServer {
    tool_router: ToolRouter<Self>,
    blocking_ops: Arc<Semaphore>,
}

impl SlugAuditServer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            blocking_ops: Arc::new(Semaphore::new(MAX_CONCURRENT_BLOCKING_OPS)),
        }
    }
}

impl Default for SlugAuditServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `work` on Tokio's blocking thread pool rather than the calling
/// async task's worker thread, after acquiring a permit that caps how many
/// such operations run concurrently. `Semaphore::acquire` only errors if
/// the semaphore has been explicitly closed, which this server never does.
async fn run_blocking<T: Send + 'static>(
    semaphore: &Semaphore,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
) -> Result<T, ErrorData> {
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ErrorData::internal_error(format!("tool task failed: {error}"), None))?
}

#[tool_router]
impl SlugAuditServer {
    #[tool(
        description = "Automatic project snapshot: file/language counts, parser failures, evidence-kind counts, open findings. No score, no risk leads."
    )]
    async fn report(
        &self,
        request: Parameters<tools::ReportRequest>,
    ) -> Result<Json<tools::ReportResponse>, ErrorData> {
        run_blocking(&self.blocking_ops, move || tools::report(&request)).await
    }

    #[tool(
        description = "Arbitrary read-only SQL against the project's own database. Search, symbol/import/diagnostic lookup, and source retrieval all reach through this one tool. Only writes are rejected, by the connection itself."
    )]
    async fn query(
        &self,
        request: Parameters<tools::QueryRequest>,
    ) -> Result<Json<tools::QueryResponse>, ErrorData> {
        run_blocking(&self.blocking_ops, move || tools::query(&request)).await
    }

    #[tool(
        description = "Tree-sitter structural pattern matching against one file, for patterns normalized evidence and query can't easily express (e.g. an S-expression query for a specific AST shape)."
    )]
    async fn structure(
        &self,
        request: Parameters<tools::StructureRequest>,
    ) -> Result<Json<tools::StructureResponse>, ErrorData> {
        run_blocking(&self.blocking_ops, move || tools::structure(&request)).await
    }

    #[tool(
        description = "Persist an AI-reviewed conclusion, tied to the file's current hash. Never generates a conclusion itself; auto-invalidates (status becomes stale) the moment the file's hash changes."
    )]
    async fn finding(
        &self,
        request: Parameters<tools::FindingRequest>,
    ) -> Result<Json<tools::FindingResponse>, ErrorData> {
        run_blocking(&self.blocking_ops, move || tools::finding(&request)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SlugAuditServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(INSTRUCTIONS)
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
