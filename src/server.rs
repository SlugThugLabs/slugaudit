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
    Implementation, InitializeResult, ListResourcesResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, RequestMetaObject,
    ServerCapabilities,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use std::sync::Arc;
use tokio::sync::Semaphore;

const INSTRUCTIONS: &str = "SlugAudit is built for you, not for humans: it is an index of this project's code — parsed \
    structure, symbols, imports, exports, call sites, comments, diagnostics — that you query instead of reading \
    files. It answers structural and aggregate questions that reading and grep cannot answer cheaply: what a \
    symbol is and where it is defined, every call site of a given callee, which project files import which, \
    per-file counts, parser diagnostics, and repo-wide denominators (\"14 call sites of `unwrap` in this file; \
    the repo median is 1\"). A question that costs tens of thousands of tokens in file reads usually costs a \
    few hundred as a query, and the tokens you don't burn re-reading files are tokens you spend finding what \
    matters. Evidence is re-synchronized on every call, so you are never reasoning about a stale copy of the \
    tree — it mirrors the files on disk, not unsaved editor buffers.\n\n\
    FIRST-WIN RULE: about to open more than ~3 files to answer one structural question — where is X defined, \
    who calls Y, what does Z import? — query instead. A cheap start on an enabled project: `report` for the \
    snapshot, then `leads` with only `path` for every mechanic and its count — two calls, no source reads. After \
    that, use whichever instrument answers each question best.\n\n\
    USE IT INSTEAD OF: grep for a symbol, callee, diagnostic, or comment marker → `query` (SQL over the index) \
    or `leads` (the same mechanics without writing SQL); cat to read a file → SELECT content FROM files WHERE \
    path = '…', or `structure` when you want AST nodes rather than text; tracing imports by hand → \
    `dependency_edges` joined in a recursive CTE (see IMPORTS); notes you want back later → `finding` (see \
    OTHER TOOLS). grep and reading a file you already know remain the right tools for a literal lookup.\n\n\
    LEADS: `leads` with only `path` returns every mechanic id with its row count, its distinct-file count \
    (`files`), and `covers` — `rust_only` or `all_languages`, the machine-readable form of \"a zero here means \
    unchecked, not absent\" — in declaration order: call sites by callee (`unwrap`, `expect`, \
    `panic`/`unreachable`, `todo`/`unimplemented`, `assert`/`assert_eq`/`assert_ne`/`debug_assert*`), \
    diagnostics, TODO/FIXME/HACK/XXX comment markers, and credential-shaped file paths. Counts are complete \
    regardless of `limit`/`offset` and carry no ordering by importance. Call again with one `mechanic` id for \
    file/line rows, optionally narrowed with `pattern` (a path GLOB like `src/auth/*`; without `*` or `?` it is \
    a substring match) — it filters the counts and the rows alike. Pass `mode: compact` to drop each row's \
    `detail` and get `mechanic`/`file`/`line` triples instead; with no `mechanic` that enumerates the whole \
    surface in one response, under the same paging fields. A rows response always carries \
    `returned`/`total` and `complete`, plus the file denominators `files_total`/`files_covered`: at scale the \
    number you can actually exhaust is the files containing the mechanic, not the rows. There is no row cap — a \
    page is cut only by your `limit` (omit it for everything that fits) and by the response byte budget, \
    reported as `row_budget_bytes`/`rows_bytes` with `limited_by` naming what stopped it. When `complete` is \
    false, page with `offset` = `next_offset` and pass `expected_revision` = the first page's `revision_id`: if \
    the index changed mid-sweep the call is refused with `stale_revision` and a remedy restarting the sweep from \
    offset 0, instead of silently skipping or repeating rows. Two limits to know. (1) The callee-based mechanics \
    read Call evidence, which only the Rust call walker produces: on a project with no Rust files those counts \
    are 0 because nothing was checked, not because nothing is there — diagnostics, comment markers, and \
    credential paths cover every indexed language. (2) Each row's `detail` is clipped to 200 characters, and \
    `detail_clipped` is true when characters were dropped; the remainder lives only in the file.\n\n\
    QUERY SCHEMA: `files` (id, path, language, byte_len, content — full text of every indexed file, including \
    non-code files), `evidence` (id, file_id, kind, start_line, end_line, payload — JSON; kinds Structure, \
    Import, Export, Comment, Docstring, Symbol, Diagnostic, Call), `dependency_edges` \
    (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled). One read-only \
    SQL statement per call, with no row cap (add your own LIMIT), cut only by the response byte budget (`truncated`/`next_offset`); \
    when resuming an `offset`, pass the previous page's `revision_id` as `expected_revision` and a changed index \
    is refused with `stale_revision` instead of returning rows from a different ordering. Always JOIN evidence \
    to files: SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = \
    'Symbol' AND json_extract(e.payload, '$.name') = 'configure'. Discover the rest with SELECT name FROM \
    sqlite_master WHERE type='table', and read `slugaudit://schema.sql`.\n\n\
    PAYLOAD KEYS: read `slugaudit://queries/evidence_payload_keys.md` for every `evidence.payload` \
    key, its JSON type, and the `evidence.kind` it belongs to (always json_extract the payload, \
    never LIKE on its text).\n\n\
    IMPORTS: `dependency_edges.resolution_kind` is Resolved, Unresolved, or External; `syntax_unmodeled` = 1 \
    marks an Unresolved edge whose import syntax the resolver could not parse at all (e.g. Go's `import \"fmt\"`), \
    as opposed to one that parsed but matched no project file. `report` gives the split as `import_resolution` \
    plus `unsupported_language_unresolved_count` — a graph that is mostly External/Unresolved makes dependency \
    walking a dead end, not a finding.\n\n\
    STRUCTURE: `structure` runs a Tree-sitter S-expression query (required, and it must name at least one \
    capture) against indexed content — in one `file`, or project-wide with `language` and an optional `pattern` \
    path filter (e.g. `src/**/*.rs`). Rust `(function_item name: (identifier) @name)`, \
    Python `(function_definition name: (identifier) @name)`. Ready-made recipes are resources: \
    `slugaudit://queries/README.md` is the index of every one of them, beside the payload-key \
    reference and `slugaudit://schema.sql`.\n\n\
    OTHER TOOLS: `report` is the project snapshot — files, languages, evidence kinds, parser failures, import \
    resolution, credential-shaped file count. `finding` persists a conclusion you have personally reviewed, \
    bound to the file's current content hash: it stales the moment that file changes, and it is visible only to \
    this session, so treat it as your notebook rather than a shared database. `finding_read` returns this \
    session's findings. `project_control` enables or disables a project — `on` also appends the runtime \
    directory `.planning/slugaudit/` to the project's root `.gitignore` when no equivalent rule is there, and \
    reports that as `gitignore.status` (`added` / `already_ignored` / `not_written`); `off` never edits that \
    file. A moved or re-extracted project's stale \
    database is discarded and rebuilt from source automatically. `health` reports watcher and database state.\n\n\
    FLOW: report → leads → drill with query/structure → read source only where evidence points → finding.\n\n\
    FAILURES: an error's `ErrorData.data` carries `code` — a stable lowercase identifier (e.g. \
    `sync_budget_exhausted`, `project_not_enabled`) — plus `remedy`, the next action to take; branch \
    on `code` instead of parsing the message, and when `remedy` names a knob or a call, do that \
    rather than retrying unchanged.\n\n\
    JUDGMENT IS YOURS: SlugAudit is not an auditor and never will be — it performs no risk detection, assigns \
    no severity, ranks nothing, and draws no conclusions. Every judgment in your response — what is buggy, how \
    severe, what to fix — is entirely yours; the index hands you mechanics, and deciding which of them is a lead \
    is your call. It records only the severity, category, and description you supply — it never generates \
    them. Never claim SlugAudit identified, rated, or recommended anything — it cannot; evidence is \
    not judgment.";

#[derive(Clone)]
/// The MCP server: generated tool router, blocking-pool semaphore, and the
/// per-server `SourceSyncManager` it serves evidence from.
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

/// The complete tool surface as it is advertised on `tools/list` — names,
/// titles, descriptions, input schemas, output schemas, and annotations,
/// sorted by name.
///
/// `#[tool_handler]` generates `list_tools` as exactly
/// `tool_router.list_all()` (rmcp 3.1.4), so this is the wire content, not a
/// reconstruction of it. `check_mcp_surface` snapshots it beside
/// `resources::list()` and fails on drift, which makes a deliberate change to
/// any of those fields a recorded change; the router is built with a
/// watcher-less manager because nothing here executes a tool.
#[must_use]
pub fn tool_surface() -> Vec<rmcp::model::Tool> {
    SlugAuditServer::with_manager(sync::SourceSyncManager::default())
        .tool_router
        .list_all()
}

#[tool_router]
impl SlugAuditServer {
    #[tool(
        title = "Project snapshot",
        description = "Automatic project snapshot: file/language counts, parser failures, evidence-kind counts, open findings, and inbound/outbound dependency-edge counts. Use this first to understand the project's shape, then query for specifics. No score, no risk leads.",
        annotations(read_only_hint = true, destructive_hint = false)
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
        title = "Mechanical leads",
        annotations(read_only_hint = true, destructive_hint = false),
        description = "Want somewhere to start? Mechanical facts extracted from the index — \
         call sites by callee name (unwrap, expect, panic, todo, unimplemented, assert), \
         diagnostics, TODO/FIXME/HACK comment markers, and credential-shaped files. Nothing here \
         is judged, ranked, scored, or ordered by importance: every item is a fact, and deciding \
         which of them matter is entirely your call. Call with only `path` to get every mechanic \
         with its row count, its distinct-file count (`files`) and `covers` (`rust_only` means a \
         zero count is unchecked, not absent), then call again with one `mechanic` id to get \
         file/line rows, optionally narrowed with `pattern` (a path GLOB, e.g. `src/auth/*`; \
         without `*`/`?` it is a substring match). A rows response always reports `returned`/`total` \
         and `complete` plus the file denominators `files_total`/`files_covered`; there is no row \
         cap, so a page is cut only by your `limit` (omit it for everything that fits) and by the \
         response byte budget (`row_budget_bytes`/`rows_bytes`, with `limited_by` naming what \
         stopped it). To page, pass `offset` = `next_offset` together with `expected_revision` = \
         the previous page's `revision_id`: a changed index is then refused with `stale_revision` \
         instead of silently skipping or repeating rows. For `mechanic`/`file`/`line` triples \
         with no per-row text, pass `mode: compact`; with only `path` it then enumerates every \
         mechanic in one response under the same completeness fields — the cheap way to see the \
         whole surface."
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
        title = "Read-only SQL query",
        annotations(read_only_hint = true, destructive_hint = false),
        description = "Query the indexed codebase with read-only SQL. Key tables: \
         files (id, path, language, byte_len, content — full text of every indexed file including non-code files), \
         evidence (id, file_id, kind, start_line, end_line, payload — Symbol, Import, Comment, Structure), \
         dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled). \
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
        title = "AST structure query",
        annotations(read_only_hint = true, destructive_hint = false),
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
        title = "Record a finding",
        annotations(read_only_hint = false, destructive_hint = false),
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
        title = "Read session findings",
        annotations(read_only_hint = true, destructive_hint = false),
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
        title = "Enable or disable a project",
        annotations(read_only_hint = false, destructive_hint = true),
        description = "Enable or disable SlugAudit for a project. Pass `action` = `\"on\"` to enable \
         a project (creates the activation directory and runs the initial import), or `\"off\"` to \
         disable it (removes the activation directory and purges its database). On `\"on\"` the \
         runtime directory `.planning/slugaudit/` is also appended to the project-root `.gitignore` \
         unless a rule that already ignores it is present; the response's `gitignore` field reports \
         `added`, `already_ignored`, or `not_written` plus the file's path. `\"off\"` never edits \
         `.gitignore`. Supply `path` to \
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
        title = "Operational health",
        annotations(read_only_hint = true, destructive_hint = false),
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
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(
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
