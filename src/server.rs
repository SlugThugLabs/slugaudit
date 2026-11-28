// slugaudit-line-exception: approved-by=split-judge; reason=Single responsibility: defining the MCP ServerHandler surface for SlugAuditServer with its initialization instructions, resource URI validation, and semaphore-bounded tool dispatch wrappers.
//! MCP server surface: the tool contracts and their registration.
//!
//! This file declares *what* the tools are and *how* they're registered
//! with rmcp. The `#[tool]` handlers themselves — their schemas,
//! descriptions, and annotations — live in `handlers`. The *execution
//! mechanics* — semaphore-bounded blocking-pool dispatch and per-call
//! tracing — live in `crate::server_runner`, and the MCP progress
//! notifications in `crate::server_progress`, so the tool list stays a
//! readable, declarative registry instead of one file mixing schemas with
//! worker-pool plumbing.

mod handlers;

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::progress::ProgressSink;
use crate::server_progress::{open_progress_session, progress_target};
use crate::server_runner::{MAX_CONCURRENT_BLOCKING_OPS, run_blocking};
use crate::sync;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    Implementation, InitializeResult, ListResourcesResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, RequestMetaObject,
    ServerCapabilities,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool_handler};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Longest resource URI accepted from a client.
const MAX_RESOURCE_URI_BYTES: usize = 8 * 1024;

const INSTRUCTIONS: &str = "SlugAudit is an index of this project's code — symbols, imports, exports, call sites, comments, \
    parser diagnostics, file text — that you query instead of reading files: where a symbol is \
    defined, every call site of a callee, which files import which, per-file counts and repo-wide \
    denominators. Reflects disk, not editor buffers; re-hashed before every call. Only Rust is \
    tested; other languages are best-effort (`report` lists them under `notices`).\n\n\
    FIRST-WIN RULE: about to open more than ~3 files to answer one structural question (where is X \
    defined, who calls Y, what does Z import)? Query instead. On an enabled project start with \
    `report`, then `leads` with only `path`: two calls, no source reads.\n\n\
    USE IT INSTEAD OF: grep for a symbol, callee, diagnostic or comment marker → `query` or `leads`; \
    cat → SELECT content FROM files WHERE path = '…', or `structure` for AST nodes; tracing imports \
    by hand → `dependency_edges` in a recursive CTE; notes for later → `finding`. grep and reading a \
    file you already know stay right for a literal lookup.\n\n\
    LINE NUMBERS: 1-based and inclusive everywhere.\n\n\
    PAGING: `leads`, `query`, `structure`, `finding_read` and `mechanic list` return a `page` \
    (`total`, `complete`, `cursor`, `page_bytes`, `limited_by`) cut by bytes; raise `page_bytes` \
    (64 KiB) up to the server ceiling. A `page.cursor` means more: repeat the call with only \
    that `cursor` for the next page of the same stored answer (no sync, unaffected by later \
    edits); none means done. A `partial` \
    `page.completeness` names `resume_after_file`: resuming is a NEW request \
    (`structure`: `start_after_file`) that syncs. `unknown_cursor` (bad, foreign, or from before \
    a restart) and `result_expired` (reclaimed): ask again without it. `result_store_full`: \
    finish open reads or narrow. `result_too_large`: add LIMIT or filters. \
    `result_store_unavailable`: paging cannot run here.\n\n\
    LEADS: with only `path`, every mechanic with its row count, `files`, and `covers` (the languages \
    it read; a missing language was not checked, so its zero means nothing). Your registered \
    mechanics come first, then built-ins: calls to unwrap, expect, panic/unreachable, \
    todo/unimplemented, assert*; diagnostics; TODO/FIXME/HACK/XXX markers; credential-shaped paths. \
    Pass one `mechanic` for rows, `pattern` (path GLOB, or a substring without `*`/`?`) to narrow, \
    `mode: compact` for file/line triples. Counts are complete and unordered; \
    `files_total`/`files_covered` are the file denominators. Callee mechanics read rust, python, \
    javascript, typescript, tsx, go, java and c calls by Rust callee names. While `files_not_parsed` > 0 every count is a floor; `report` \
    lists those files.\n\n\
    MECHANICS: the built-ins are counting primitives, not a checklist; what counts as a signal is \
    your decision. `mechanic` `action: add` registers `id`, `language`, `query` (Tree-sitter, \
    exactly one capture) and `note`, and returns `first_count`; `leads` then counts it like a \
    built-in. Entries last for this session. A built-in is never removed or overridden.\n\n\
    QUERY: one read-only SELECT/WITH per call over `files` (id, path, language, byte_len, content), \
    `evidence` (id, file_id, kind, start_line, end_line, payload JSON; kinds Structure, Import, \
    Export, Comment, Docstring, Symbol, Diagnostic, Call) and `dependency_edges` (from_file_id, \
    to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled). JOIN evidence to \
    files on file_id; read payload with json_extract, never LIKE (keys: \
    `slugaudit://queries/evidence_payload_keys.md`). Add LIMIT when exploring; some clients fail on \
    huge results. An oversized first row is clipped, with `substr` continuations in `clipped_cells`. \
    `line_count(text)` and `line_range(text, start, end)` read files by line. PRAGMA statements are \
    refused; pragma_table_info and the other introspection functions work. `resolution_kind` is \
    Resolved, Unresolved or External (`syntax_unmodeled` = 1: import syntax not parsed); \
    `report.import_resolution` gives the split. Recipes and schema: `slugaudit://queries/README.md`, \
    `slugaudit://schema.sql`.\n\n\
    GRAMMARS: Rust is tested; other languages are best-effort and flagged in `notices`. \
    Markdown, reStructuredText, AsciiDoc, Org, plain-text and ignore files are \
    `language = 'prose'`: stored, never parsed.\n\n\
    OTHER TOOLS: `report` is the project snapshot. `structure` runs a Tree-sitter query (at least \
    one capture) on one `file` or a whole `language`. `finding` records a conclusion you reviewed, \
    bound to the file hash (stale on change) and lasting this session; `finding_read` \
    reads them, `finding_clear` deletes them. `project_control` `on` creates `.planning/slugaudit/` and indexes; `off` deletes it. `health` reports phase and next action; `verbose: true` adds detail.\n\n\
    FLOW: report → leads → mechanic add → query/structure → read source only where evidence \
    points → finding.\n\n\
    FAILURES: errors carry `data.code` (e.g. `project_not_enabled`, `sql_error`, `unknown_language`) and \
    `remedy`, the next action. Branch on `code`; follow `remedy`, don't retry unchanged.\n\n\
    JUDGMENT IS YOURS: SlugAudit reports evidence and never judges, rates or ranks it; the severity \
    and category on a finding are the ones you supply. Never claim SlugAudit identified, rated or \
    recommended anything.";

#[derive(Clone)]
/// The MCP server: generated tool router, blocking-pool semaphore, and the
/// per-server `SourceSyncManager` it serves evidence from.
pub struct SlugAuditServer {
    tool_router: ToolRouter<Self>,
    blocking_ops: Arc<Semaphore>,
    /// Separate semaphore for `health`, independent of `blocking_ops`, so
    /// health checks never block behind saturated sync permits while still
    /// running through `server_runner::run_blocking`.
    health_ops: Arc<Semaphore>,
    /// Per-server `SourceSyncManager`: the last-sync timestamps, session
    /// mechanics, and the paged-result store.
    manager: sync::SourceSyncManager,
}

impl SlugAuditServer {
    /// Constructs the production server: one manager, no external handles.
    #[must_use]
    pub fn new() -> Self {
        Self::with_manager(sync::SourceSyncManager::default())
    }

    /// Constructs a server with an explicit `SourceSyncManager`.
    #[must_use]
    pub(crate) fn with_manager(manager: sync::SourceSyncManager) -> Self {
        Self {
            tool_router: Self::tool_router(),
            blocking_ops: Arc::new(Semaphore::new(MAX_CONCURRENT_BLOCKING_OPS)),
            health_ops: Arc::new(Semaphore::new(MAX_CONCURRENT_BLOCKING_OPS)),
            manager,
        }
    }

    /// Shared tool-call plumbing: derive the caller's progress target,
    /// build the inner progress sink, clone the manager, and run the
    /// tool's blocking work under `semaphore`.
    async fn dispatch_with_semaphore<T: Send + 'static>(
        &self,
        semaphore: Arc<Semaphore>,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        tool_name: &'static str,
        work: impl FnOnce(Arc<dyn ProgressSink>, sync::SourceSyncManager) -> Result<T, ErrorData>
        + Send
        + 'static,
    ) -> Result<T, ErrorData> {
        let target = progress_target(&meta, &peer);
        let (progress, inner_sink) = open_progress_session(target);
        let manager = self.manager.clone();
        run_blocking(
            semaphore,
            tool_name,
            move || work(inner_sink, manager),
            progress,
        )
        .await
    }

    /// Shared tool-call plumbing for standard tools under `blocking_ops`.
    async fn dispatch<T: Send + 'static>(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        tool_name: &'static str,
        work: impl FnOnce(Arc<dyn ProgressSink>, sync::SourceSyncManager) -> Result<T, ErrorData>
        + Send
        + 'static,
    ) -> Result<T, ErrorData> {
        self.dispatch_with_semaphore(Arc::clone(&self.blocking_ops), meta, peer, tool_name, work)
            .await
    }

    /// Runs `health` through `run_blocking` under its own dedicated semaphore.
    pub(crate) async fn dispatch_health<T: Send + 'static>(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        work: impl FnOnce(Arc<dyn ProgressSink>, sync::SourceSyncManager) -> Result<T, ErrorData>
        + Send
        + 'static,
    ) -> Result<T, ErrorData> {
        self.dispatch_with_semaphore(Arc::clone(&self.health_ops), meta, peer, "health", work)
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
#[must_use]
pub fn tool_surface() -> Vec<rmcp::model::Tool> {
    SlugAuditServer::with_manager(sync::SourceSyncManager::default())
        .tool_router
        .list_all()
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
        .with_server_info(Implementation::new("slugaudit", env!("CARGO_PKG_VERSION")))
    }

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
        read_resource_contents(&request.uri).map(Into::into)
    }
}

/// Resolves one resource URI from the embedded catalogue.
fn read_resource_contents(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    if crate::util::exceeds_input_bytes(uri, MAX_RESOURCE_URI_BYTES) {
        return Err(invalid_params_with_code(
            format!("resource uri exceeds {MAX_RESOURCE_URI_BYTES} bytes"),
            ErrorCode::InvalidInput,
        ));
    }
    crate::resources::read(uri)
        .map(|contents| ReadResourceResult::new(vec![contents]))
        .ok_or_else(|| ErrorData::resource_not_found(format!("unknown resource `{uri}`"), None))
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
