use crate::tools;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};

const INSTRUCTIONS: &str = "SlugAudit supplies searchable, trustworthy evidence about a codebase — \
    parsed structure, symbols, imports, and prior AI-reviewed findings — so an audit never has to \
    re-derive facts by reading flat files. Use `report` for an automatic snapshot of what evidence \
    exists, `query` for arbitrary read-only SQL against the project's own database (search, symbol/ \
    import/diagnostic lookup, dependency traversal, and source retrieval all reach through it), \
    `structure` for Tree-sitter structural pattern matching, and `finding` to persist a conclusion \
    you have actually reviewed. SlugAudit performs no automated risk detection and reaches no \
    conclusions itself: it supplies evidence, the calling AI performs all judgment.";

#[derive(Clone)]
pub struct SlugAuditServer {
    tool_router: ToolRouter<Self>,
}

impl SlugAuditServer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for SlugAuditServer {
    fn default() -> Self {
        Self::new()
    }
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
        tools::report(&request)
    }

    #[tool(
        description = "Arbitrary read-only SQL against the project's own database. Search, symbol/import/diagnostic lookup, dependency traversal (recursive CTEs), and source retrieval all reach through this one tool. Only writes are rejected, by the connection itself."
    )]
    async fn query(
        &self,
        request: Parameters<tools::QueryRequest>,
    ) -> Result<Json<tools::QueryResponse>, ErrorData> {
        tools::query(&request)
    }

    #[tool(
        description = "Tree-sitter structural pattern matching against one file, for patterns normalized evidence and query can't easily express (e.g. an S-expression query for a specific AST shape)."
    )]
    async fn structure(
        &self,
        request: Parameters<tools::StructureRequest>,
    ) -> Result<Json<tools::StructureResponse>, ErrorData> {
        tools::structure(&request)
    }

    #[tool(
        description = "Persist an AI-reviewed conclusion, tied to the file's current hash. Never generates a conclusion itself; auto-invalidates (status becomes stale) the moment the file's hash changes."
    )]
    async fn finding(
        &self,
        request: Parameters<tools::FindingRequest>,
    ) -> Result<Json<tools::FindingResponse>, ErrorData> {
        tools::finding(&request)
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
