// slugaudit-line-exception: approved-by=slugthug; reason=the MCP server surface declares all tool contracts, schema descriptions, and rmcp dispatch handlers in one cohesive registry
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
    Implementation, ListResourcesResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, RequestMetaObject,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use std::sync::Arc;
use tokio::sync::Semaphore;

const INSTRUCTIONS: &str = "SlugAudit is built for you, not for humans: it is an index of a project's code — parsed \
    structure, symbols, imports, call sites, comments, and diagnostics — that you query instead of \
    reading files. The point is budget. A search that costs tens of thousands of tokens in file \
    reads costs a few hundred as a query, and the tokens you don't burn re-reading files are tokens \
    you spend finding what actually matters. \
    \
    It is not an auditor and never will be: it performs no risk detection, assigns no severity, \
    ranks nothing, and draws no conclusions. Every judgment in your response — what is buggy, how \
    severe, what to fix — is entirely yours. SlugAudit hands you mechanics; deciding which of them \
    is a lead is your call. \
    \
    FIRST MOVE: call `report` for the project snapshot (files, languages, evidence kinds, parser \
    failures, import resolution), then call `leads` with only `path` for a count of every mechanic \
    it can surface — call sites by callee (`unwrap`, `expect`, `panic`, `todo`, `unimplemented`, \
    `assert`), diagnostics, TODO/FIXME/HACK comment markers, credential-shaped files. Those counts \
    are unranked, so pick whichever you think matters and call `leads` again with that `mechanic` \
    for file/line rows, paging with `limit`/`offset`. Callee-based mechanics are Rust-only in this \
    revision. \
    \
    WHAT THIS ADDS: structural and aggregate questions that reading or grep can't answer cheaply — \
    what a symbol is and where it is defined, every call site of a given callee, which project files \
    import which, per-file counts, parser diagnostics, and repo-wide denominators (\"14 call sites of \
    `unwrap` in this file; the repo median is 1\"). Evidence is re-synchronized on every call, so you \
    are never reasoning about a stale copy of the tree — it mirrors the files on disk, not unsaved \
    editor buffers. grep and reading files remain the right tools for a literal lookup; this is one \
    more instrument on your belt, not a replacement for the ones you already have. \
    \
    TRY IT FIRST: on a project where the index is already enabled, spend two calls on `report` and \
    `leads` before your first file read. It costs almost nothing and shows you what this index can \
    answer here. Then use whichever instrument answers each question best — grep, reading, or this. \
    Nobody is asking you to prefer SlugAudit. \
    \
    QUERY SCHEMA: `files` (id, path, language, byte_len, content — full text of every indexed file, \
    including non-code files), `evidence` (id, file_id, kind, start_line, end_line, payload — JSON; \
    kinds Structure, Import, Export, Comment, Docstring, Symbol, Diagnostic, Chunk, RawNode, Metric, \
    Call), `dependency_edges` (from_file_id, to_file_id, resolution_kind, syntax_unmodeled). Always \
    JOIN evidence to files: SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = \
    f.id WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure'. Discover the \
    rest with SELECT name FROM sqlite_master WHERE type='table', and read `slugaudit://schema.sql`. \
    \
    PAYLOAD KEYS (evidence.payload holds JSON — always json_extract, never LIKE on payload text): \
    Symbol {name, kind, type_annotation, doc}; Structure {kind, name, visibility, decorators, \
    signature}; Import {source, items, alias, is_wildcard}; Export {name, kind}; Comment {text, \
    kind, associated_node}; Docstring {text, format, associated_item}; Diagnostic {message, \
    severity}; Call {language, callee_text, callee_name, caller, resolution, confidence} (Rust only). \
    \
    IMPORTS: dependency_edges.resolution_kind is Resolved, Unresolved, or External, and \
    syntax_unmodeled = 1 marks edges whose import syntax the resolver could not parse at all. \
    `report.import_resolution` gives you the ratio before you lean on traversal — a graph that is \
    mostly External/Unresolved makes dependency walking a dead end, not a finding. \
    \
    STRUCTURE: `structure` runs Tree-sitter queries against the same indexed content — Rust \
    (function_item name: (identifier) @name body: (block) @body), Python (function_definition name: \
    (identifier) @name), or project-wide with `language` + `pattern`. Ready-made recipes are \
    resources: `slugaudit://queries/README.md`, `slugaudit://queries/find_unhandled_panics.sql`, \
    `slugaudit://queries/auth_attack_surface.sql`, `slugaudit://queries/syntax_diagnostics.sql`, \
    `slugaudit://queries/rust_functions.scm`. \
    \
    RECORD: `finding` persists a conclusion you have personally reviewed, bound to the file's \
    current content hash — it stales the moment that file changes, and it is visible only to this \
    session, so treat it as your notebook rather than a shared database. `finding_read` returns this \
    session's findings. `project_control` enables or disables a project; `health` reports watcher and \
    database state. \
    \
    FLOW: report → leads → drill with query/structure → read source only where evidence points → \
    finding. \
    \
    WHEN NOT TO USE THIS: a `LIKE` over `files.content` is the worst of both worlds — index-build \
    cost for grep-quality output. Ask the structured question instead, or use grep. Never claim \
    SlugAudit identified, rated, or recommended anything — it cannot; evidence is not judgment.";

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
    /// so the tool handlers don't each repeat this prologue (DRY).
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
        description = "Want somewhere to start? Mechanical facts extracted from the index — \
         call sites by callee name (unwrap, expect, panic, todo, unimplemented, assert), \
         diagnostics, TODO/FIXME/HACK comment markers, and credential-shaped files. Nothing here \
         is judged, ranked, scored, or ordered by importance: every item is a fact, and deciding \
         which of them matter is entirely your call. Call with only `path` to get a count per \
         mechanic, then call again with one `mechanic` id to get file/line rows, paging with \
         `limit`/`offset`. Callee-based mechanics cover Rust only in this revision (the call \
         walker is Rust-only); diagnostics, comment markers, and credential files cover every \
         indexed language."
    )]
    async fn leads(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::LeadsRequest>,
    ) -> Result<Json<tools::LeadsResponse>, ErrorData> {
        self.dispatch(meta, peer, "leads", move |sink, manager| {
            tools::leads(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        description = "Query the indexed codebase with read-only SQL. Key tables: \
         files (id, path, language, byte_len, content — full text of every indexed file including non-code files), \
         evidence (id, file_id, kind, start_line, end_line, payload — Symbol, Import, Comment, Structure), \
         dependency_edges (from_file_id, to_file_id, kind, status). \
         To inspect code or file content directly from the DB without filesystem reads, query `files.content` (e.g. `SELECT content FROM files WHERE path = 'src/main.rs'`). \
         To link evidence to file paths, always JOIN: `SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure'`. \
         Schema discovery: `SELECT name FROM sqlite_master WHERE type='table'`."
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
        description = "Inspect and extract code AST structures from a file or across the project using Tree-sitter. \
         Specify `file` to search a single file, or omit `file` and specify `language` (with optional `pattern`) \
         to search across matching project files. Returns matched AST nodes with `file`, `start_line`, `end_line`, \
         and `text` snippet. The `query` must name at least one capture with `@name`. \
         Examples: Rust `(function_item name: (identifier) @name body: (block) @body)`, \
         Python `(function_definition name: (identifier) @name body: (block) @body)`, \
         TypeScript/JS `(function_declaration name: (identifier) @name)`, \
         or whole-node `(class_definition) @cls`."
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
        self.dispatch(meta, peer, "project_control", move |sink, manager| {
            tools::project_control(&request, sink.as_ref(), Some(&manager))
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
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_protocol_version(ProtocolVersion::LATEST)
        .with_instructions(INSTRUCTIONS)
        // Without this, rmcp fills `serverInfo` from its own build
        // environment and every MCP client shows the connected server
        // as "rmcp" (the library's name/version) instead of SlugAudit.
        .with_server_info(Implementation::new("slugaudit", env!("CARGO_PKG_VERSION")))
    }

    /// The embedded cookbook and schema. Pull-based: an agent that never
    /// reads them pays nothing for them.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(crate::resources::list()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        crate::resources::read(&request.uri)
            .map(|contents| ReadResourceResult::new(vec![contents]).into())
            .ok_or_else(|| {
                ErrorData::resource_not_found(format!("unknown resource `{}`", request.uri), None)
            })
    }
}
