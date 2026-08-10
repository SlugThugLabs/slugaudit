//! MCP server surface: the tool contracts and their registration.
//!
//! This file declares *what* the tools are (their JSON schemas and the
//! thin handlers that call into `crate::tools`) and *how* they're
//! registered with rmcp. The *execution mechanics* — semaphore-bounded
//! blocking-pool dispatch, per-call tracing, and MCP progress
//! notifications — live in [`crate::server_runner`], so the tool list
//! stays a readable, declarative registry instead of a 280-line file
//! mixing schemas with worker-pool plumbing.

use crate::server_runner::{
    MAX_CONCURRENT_BLOCKING_OPS, build_inner_sink, progress_target, run_blocking,
};
use crate::tools;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Meta, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use std::sync::Arc;
use tokio::sync::Semaphore;

const INSTRUCTIONS: &str = "SlugAudit supplies searchable, trustworthy evidence about a codebase — \
    parsed structure, symbols, imports, diagnostics, and prior AI-reviewed findings. Use `report` \
    for an automatic snapshot of what evidence exists, `query` for arbitrary read-only SQL against \
    the project's own database (search, symbol/import/diagnostic lookup, dependency traversal via \
    recursive CTEs over dependency_edges, and source retrieval all reach through it), `structure` \
    for Tree-sitter structural pattern matching, and `finding` to persist a conclusion you have \
    actually reviewed. Use `project_control` with `action` = `\"on\"` to enable a project (creates \
    the activation directory and runs the first import) or `\"off\"` to disable it. \
    SlugAudit performs no automated risk detection and reaches no conclusions itself: it supplies \
    evidence, the calling AI performs all judgment.";

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

#[tool_router]
impl SlugAuditServer {
    #[tool(
        description = "Automatic project snapshot: file/language counts, parser failures, evidence-kind counts, open findings. No score, no risk leads."
    )]
    async fn report(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::ReportRequest>,
    ) -> Result<Json<tools::ReportResponse>, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "report",
            move || tools::report(&request, inner_sink.as_ref()),
            progress,
        )
        .await
    }

    #[tool(
        description = "Arbitrary read-only SQL against the project's own database. Search, symbol/import/diagnostic lookup, and source retrieval all reach through this one tool. Only writes are rejected, by the connection itself."
    )]
    async fn query(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::QueryRequest>,
    ) -> Result<Json<tools::QueryResponse>, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "query",
            move || tools::query(&request, inner_sink.as_ref()),
            progress,
        )
        .await
    }

    #[tool(
        description = "Tree-sitter structural pattern matching against one file, for patterns normalized evidence and query can't easily express (e.g. an S-expression query for a specific AST shape)."
    )]
    async fn structure(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::StructureRequest>,
    ) -> Result<Json<tools::StructureResponse>, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "structure",
            move || tools::structure(&request, inner_sink.as_ref()),
            progress,
        )
        .await
    }

    #[tool(
        description = "Persist a conclusion you have personally reviewed about a specific file \
         and line range. Supply `path` (any path in the active project, to select the database), \
         `file` (project-relative path of the file), `line_start`/`line_end` (one-based inclusive), \
         plus `severity`, `category`, `title`, and `description` (all your own judgment — SlugAudit \
         never generates these). The finding is bound to the file's current content hash and \
         auto-invalidates (status becomes stale) the moment that file changes, so findings never \
         outlive the source they were about. Use this to record reviewed issues, not raw diagnostics."
    )]
    async fn finding(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::FindingRequest>,
    ) -> Result<Json<tools::FindingResponse>, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "finding",
            move || tools::finding(&request, inner_sink.as_ref()),
            progress,
        )
        .await
    }

    #[tool(
        description = "Enable or disable SlugAudit for a project. Pass `action` = `\"on\"` to enable \
         a project (creates the activation directory and runs the initial import), or `\"off\"` to \
         disable it (removes the activation directory and purges its database). Supply `path` to \
         target a specific project root; defaults to the current directory."
    )]
    async fn project_control(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::ProjectControlRequest>,
    ) -> Result<Json<tools::ProjectControlResponse>, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "project_control",
            move || tools::project_control(&request, inner_sink.as_ref()),
            progress,
        )
        .await
    }

    #[tool(
        description = "Operational snapshot: watcher health, unreconciled event counts, last \
         verified sequence, current revision id and file count, parser pack version, and \
         process-wide tool-call counters. Intended for operators/health checks; safe to call \
         repeatedly and never mutates state. Call `project_control` with `\"on\"` on a path first \
         to surface per-project database state; calling without a path returns global counters \
         and the most-recently-touched project's watcher health only."
    )]
    async fn health(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::HealthRequest>,
    ) -> Result<Json<tools::HealthResponse>, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "health",
            move || tools::health(&request, inner_sink.as_ref()),
            progress,
        )
        .await
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
