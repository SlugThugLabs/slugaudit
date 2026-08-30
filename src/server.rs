//! MCP server surface: the tool contracts and their registration.
//!
//! This file declares *what* the tools are (their JSON schemas and the
//! thin handlers that call into `crate::tools`) and *how* they're
//! registered with rmcp. The *execution mechanics* — semaphore-bounded
//! blocking-pool dispatch, per-call tracing, and MCP progress
//! notifications — live in [`crate::server_runner`], so the tool list
//! stays a readable, declarative registry instead of a 280-line file
//! mixing schemas with worker-pool plumbing.

use crate::progress::ProgressSink;
use crate::server_runner::{
    MAX_CONCURRENT_BLOCKING_OPS, build_inner_sink, progress_target, run_blocking,
};
use crate::sync;
use crate::tools;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Implementation, ProtocolVersion, RequestMetaObject, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use std::sync::Arc;
use tokio::sync::Semaphore;

const INSTRUCTIONS: &str = "SlugAudit does not audit. It is not an auditor and never will be: it \
    performs no risk detection, assigns no severity, draws no conclusions, and offers no \
    recommendations. Every judgment in your response — what is buggy, how severe, what to fix — \
    is entirely yours. SlugAudit's only job is to supply searchable, trustworthy evidence about a \
    codebase: parsed structure, symbols, imports, diagnostics, and prior AI-authored findings. \
    \
    WORKFLOW: Always query first, then read files only for what the queries surface. \
    Use `report` for a quick snapshot (file count, languages, evidence kinds, open findings). \
    Then use `query` to locate exactly which files and lines matter before reading anything. \
    \
    QUERY SCHEMA: Key tables are `files` (every source file with path, language, content), \
    `evidence` (Symbol, Import, Comment, Structure extracted from each file — payload is JSON, \
    use json_extract), and `dependency_edges` (import resolution: Resolved/Unresolved/External). \
    Schema discovery: SELECT name FROM sqlite_master WHERE type='table'. \
    Example: SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id \
    WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure'. \
    \
    Use `structure` for Tree-sitter structural pattern matching, `finding` to persist \
    a conclusion you have actually reviewed, and `finding_read` to retrieve findings scoped to \
    the current agent session. Use `project_control` with `action` = `\"on\"` to enable a \
    project or `\"off\"` to disable it. Never claim SlugAudit identified, rated, or \
    recommended anything — it cannot; evidence is not judgment.";

#[derive(Clone)]
pub struct SlugAuditServer {
    tool_router: ToolRouter<Self>,
    blocking_ops: Arc<Semaphore>,
    /// Per-server `SourceSyncManager`. Replaces the `static
    /// SYNC_MANAGER: OnceLock<…>` global that previously sat in
    /// `tools::context`; each server now owns its own watcher state,
    /// ignore rules, and `last_sync_unix_seconds` counter, so two
    /// servers in the same process would track independent projects.
    /// `Clone` is the cheap clone of `SourceSyncManager` itself — the
    /// inner `WatchManager` is shared across `Clone`s of the server.
    manager: sync::SourceSyncManager,
}

impl SlugAuditServer {
    /// Constructs a server with a real `notify` watcher (when supported
    /// on the host platform). Production entry point; tests that don't
    /// need watcher-based incremental reconcile should use
    /// [`SlugAuditServer::with_manager`] with a `SourceSyncManager::default()`
    /// so the watcher is bypassed and the tests can't leak `notify`
    /// handles between cases.
    #[must_use]
    pub fn new() -> Self {
        Self::with_manager(sync::SourceSyncManager::with_watcher())
    }

    /// Constructs a server with an explicit `SourceSyncManager`. The
    /// composition root for tests and for any caller that wants to
    /// inject a mock / custom manager.
    #[must_use]
    pub fn with_manager(manager: sync::SourceSyncManager) -> Self {
        Self {
            tool_router: Self::tool_router(),
            blocking_ops: Arc::new(Semaphore::new(MAX_CONCURRENT_BLOCKING_OPS)),
            manager,
        }
    }

    /// Shared tool-call plumbing: derive the caller's progress target,
    /// build the inner progress sink, clone the manager, and run the
    /// tool's blocking work under the semaphore. Each `#[tool]` handler
    /// stays declarative — it only names the tool and its work closure,
    /// so the six handlers don't each repeat this prologue (DRY).
    async fn dispatch<T: Send + 'static>(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        tool_name: &'static str,
        work: impl FnOnce(Arc<dyn ProgressSink>, sync::SourceSyncManager) -> Result<T, ErrorData>
        + Send
        + 'static,
    ) -> Result<T, ErrorData> {
        let progress = progress_target(meta, &peer);
        let inner_sink = build_inner_sink(progress.clone());
        let manager = self.manager.clone();
        run_blocking(
            Arc::clone(&self.blocking_ops),
            tool_name,
            move || work(inner_sink, manager),
            progress,
        )
        .await
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
        description = "Automatic project snapshot: file/language counts, parser failures, evidence-kind counts, open findings. Use this first to understand the project's shape, then query for specifics. No score, no risk leads."
    )]
    async fn report(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::ReportRequest>,
    ) -> Result<Json<tools::ReportResponse>, ErrorData> {
        self.dispatch(meta, peer, "report", move |sink, manager| {
            tools::report(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        description = "Query the indexed codebase with read-only SQL. Key tables: files (every source file with path, language, content), evidence (Symbol, Import, Comment, Structure extracted from each file — use json_extract on the payload column), dependency_edges (import resolution graph with Resolved/Unresolved/External status). Always query first to find which files and lines matter, then read only those. Example: SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure'. Schema discovery: SELECT name FROM sqlite_master WHERE type='table'."
    )]
    async fn query(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::QueryRequest>,
    ) -> Result<Json<tools::QueryResponse>, ErrorData> {
        self.dispatch(meta, peer, "query", move |sink, manager| {
            tools::query(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        description = "Tree-sitter structural pattern matching against one file, for patterns normalized evidence and query can't easily express (e.g. an S-expression query for a specific AST shape). The `query` must name at least one capture with `@name` — a bare node pattern like `(function_definition)` returns an error, not results. Examples: Python `(function_definition name: (identifier) @name)`, Rust `(function_item name: (identifier) @name)`, or whole-node `(class_definition) @cls`."
    )]
    async fn structure(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::StructureRequest>,
    ) -> Result<Json<tools::StructureResponse>, ErrorData> {
        self.dispatch(meta, peer, "structure", move |sink, manager| {
            tools::structure(&request, sink.as_ref(), &manager)
        })
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
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::FindingRequest>,
    ) -> Result<Json<tools::FindingResponse>, ErrorData> {
        self.dispatch(meta, peer, "finding", move |sink, manager| {
            tools::finding(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        description = "Retrieve findings scoped to the current agent session — only returns \
         findings written by this session, never another session's conclusions. Safer than \
         querying the findings table directly through `query`, which returns every session's \
         rows. Supply `path` (any path in the active project) to select the database, and \
         optionally `file` to filter to a specific file."
    )]
    async fn finding_read(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::FindingReadRequest>,
    ) -> Result<Json<tools::FindingReadResponse>, ErrorData> {
        self.dispatch(meta, peer, "finding_read", move |sink, manager| {
            tools::finding_read(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        description = "Enable or disable SlugAudit for a project. Pass `action` = `\"on\"` to enable \
         a project (creates the activation directory and runs the initial import), or `\"off\"` to \
         disable it (removes the activation directory and purges its database). Supply `path` to \
         target a specific project root; defaults to the current directory. If the project was \
         moved, copied, or re-extracted elsewhere (e.g. unzipped fresh from an archive), a stale \
         `.planning/slugaudit/project.db` is discarded and rebuilt from source automatically — the \
         agent does not need to delete it or re-enable manually."
    )]
    async fn project_control(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::ProjectControlRequest>,
    ) -> Result<Json<tools::ProjectControlResponse>, ErrorData> {
        self.dispatch(meta, peer, "project_control", move |sink, _manager| {
            tools::project_control(&request, sink.as_ref())
        })
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
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::HealthRequest>,
    ) -> Result<Json<tools::HealthResponse>, ErrorData> {
        self.dispatch(meta, peer, "health", move |sink, manager| {
            tools::health(&request, sink.as_ref(), &manager)
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SlugAuditServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(INSTRUCTIONS)
            // Without this, rmcp fills `serverInfo` from its own build
            // environment and every MCP client shows the connected server
            // as "rmcp" (the library's name/version) instead of SlugAudit.
            .with_server_info(Implementation::new("slugaudit", env!("CARGO_PKG_VERSION")))
    }
}
