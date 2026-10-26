//! MCP server surface: the tool contracts and their registration.
//!
//! This file declares *what* the tools are and *how* they're registered
//! with rmcp. The `#[tool]` handlers themselves — their schemas,
//! descriptions, and annotations — live in `handlers`. The *execution
//! mechanics* — semaphore-bounded blocking-pool dispatch, per-call tracing,
//! and MCP progress notifications — live in `crate::server_runner`, so the
//! tool list stays a readable, declarative registry instead of one file
//! mixing schemas with worker-pool plumbing.

mod handlers;

use crate::progress::ProgressSink;
use crate::server_runner::{
    MAX_CONCURRENT_BLOCKING_OPS, build_inner_sink, progress_target, run_blocking,
};
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

/// Longest resource URI accepted from a client. Resource URIs are a fixed,
/// compile-time catalogue with no templates, so any longer value is unknown by
/// construction; capping it keeps a multi-megabyte URI out of the
/// `resource_not_found` message instead of echoing it back. rmcp's
/// `JsonRpcMessageCodec` sets no length limit of its own, unlike tool inputs,
/// which carry their own caps.
const MAX_RESOURCE_URI_BYTES: usize = 8 * 1024;

const INSTRUCTIONS: &str = "SlugAudit is an index of this project's code — symbols, imports, exports, call sites, comments, \
    parser diagnostics, file text — that you query instead of reading files: where a symbol is \
    defined, every call site of a callee, which files import which, per-file counts and repo-wide \
    denominators. Each call re-syncs from disk (not editor buffers) first.\n\n\
    FIRST-WIN RULE: about to open more than ~3 files to answer one structural question (where is X \
    defined, who calls Y, what does Z import)? Query instead. On an enabled project start with \
    `report`, then `leads` with only `path`: two calls, no source reads.\n\n\
    USE IT INSTEAD OF: grep for a symbol, callee, diagnostic or comment marker → `query` or `leads`; \
    cat → SELECT content FROM files WHERE path = '…', or `structure` for AST nodes; tracing imports \
    by hand → `dependency_edges` in a recursive CTE; notes for later → `finding`. grep and reading a \
    file you already know stay right for a literal lookup.\n\n\
    LINE NUMBERS: 1-based and inclusive everywhere.\n\n\
    PAGING: `leads`, `query`, `structure` and `finding_read` return one `page`: `returned`, `total` \
    (null when uncountable), `complete`, `next_offset` (null when done), `page_bytes` (64 KiB \
    default; pass more, up to the server ceiling), `bytes_used`, `limited_by` (`none`, `limit`, \
    `page_bytes`, `ceiling`, `match_cap`, `scan_window`, `time_budget`; more `page_bytes` lifts \
    neither `ceiling` nor `match_cap`) and `revision_id`. Pages are cut by bytes, never a row cap. \
    Resume with `offset` = `page.next_offset` and `expected_revision` = `page.revision_id`; a \
    changed index is refused with `stale_revision` (restart from offset 0) instead of skipping or \
    repeating rows. `structure` has a second cursor: when `complete` is false and `next_offset` is \
    null, repeat with `start_after_file` = `next_start_after_file` and `offset` 0.\n\n\
    LEADS: with only `path`, every mechanic with its row count, `files`, and `covers` (the languages \
    it read; a missing language was not checked, so its zero means nothing). Your registered \
    mechanics come first, then built-ins: calls to unwrap, expect, panic/unreachable, \
    todo/unimplemented, assert*; diagnostics; TODO/FIXME/HACK/XXX markers; credential-shaped paths. \
    Pass one `mechanic` for rows, `pattern` (path GLOB, or a substring without `*`/`?`) to narrow, \
    `mode: compact` for file/line triples. Counts are complete and unordered; \
    `files_total`/`files_covered` are the file denominators. Callee mechanics read rust, python, \
    javascript, typescript, tsx, go, java and c calls by Rust callee names. `detail` is clipped at \
    200 chars (`detail_clipped`). While `files_not_parsed` > 0 every count is a floor; `report` \
    lists those files.\n\n\
    MECHANICS: the built-ins are counting primitives, not a checklist; what counts as a signal is \
    your decision. `mechanic` `action: add` registers `id`, `language`, `query` (Tree-sitter, \
    exactly one capture) and `note`, and returns `first_count`; `leads` then counts it like a \
    built-in. Entries last for this session; `persist: true` saves them to \
    `.planning/slugaudit/config.json`. A built-in is never removed or overridden.\n\n\
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
    GRAMMARS: grammars are fetched on first use into `~/.cache/tree-sitter-language-pack/`. Offline, \
    those files stay searchable by `content` but have no evidence until a retry (after 10 minutes or \
    a restart); the first failed fetch can block a call for about 60 s. Markdown, reStructuredText, \
    AsciiDoc, Org, plain-text and ignore-list files are `language = 'prose'`: stored, never parsed.\n\n\
    OTHER TOOLS: `report` is the project snapshot. `structure` runs a Tree-sitter query (at least \
    one capture) on one `file` or a whole `language`. `finding` records a conclusion you reviewed, \
    bound to the file's content hash (stale once it changes), visible only to this session; \
    `finding_read` reads them. `project_control` `on` enables and indexes a project, `off` disables \
    it. `health` reports phase and next action; `verbose: true` adds operator detail.\n\n\
    FLOW: report → leads → mechanic add for shapes you decide matter → query/structure → read source \
    only where evidence points → finding.\n\n\
    FAILURES: errors carry `data.code` (e.g. `project_not_enabled`) and `remedy`, the next action. \
    Branch on `code`; do what `remedy` says instead of retrying unchanged.\n\n\
    JUDGMENT IS YOURS: SlugAudit reports evidence and never judges, rates or ranks it; the severity \
    and category on a finding are the ones you supply. Never claim SlugAudit identified, rated or \
    recommended anything.";

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
    /// on the host platform). Production entry point; crate-internal tests
    /// that don't need watcher-based incremental reconcile should use
    /// `SlugAuditServer::with_manager` with a `SourceSyncManager::default()`
    /// so the watcher is bypassed and the tests can't leak `notify`
    /// handles between cases.
    #[must_use]
    pub fn new() -> Self {
        Self::with_manager(sync::SourceSyncManager::with_watcher())
    }

    /// Constructs a server with an explicit `SourceSyncManager`. Used by
    /// [`SlugAuditServer::new`] and by `tool_surface`; not part of the
    /// advertised MCP surface.
    #[must_use]
    pub(crate) fn with_manager(manager: sync::SourceSyncManager) -> Self {
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
        read_resource_contents(&request.uri).map(Into::into)
    }
}

/// Resolves one resource URI from the embedded catalogue.
///
/// A URI longer than [`MAX_RESOURCE_URI_BYTES`] is refused as invalid
/// parameters before the catalogue lookup, so an oversized caller value is
/// never formatted into an error message. Every URI the catalogue does not
/// serve is `resource_not_found`, including a short but unknown one.
fn read_resource_contents(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    if crate::util::exceeds_input_bytes(uri, MAX_RESOURCE_URI_BYTES) {
        return Err(ErrorData::invalid_params(
            format!("resource uri exceeds {MAX_RESOURCE_URI_BYTES} bytes"),
            None,
        ));
    }
    crate::resources::read(uri)
        .map(|contents| ReadResourceResult::new(vec![contents]))
        .ok_or_else(|| ErrorData::resource_not_found(format!("unknown resource `{uri}`"), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every session pays for INSTRUCTIONS in context, so it stays short;
    /// detail belongs in the tool descriptions, schemas and resources.
    #[test]
    fn the_initialize_instructions_stay_under_their_size_budget() {
        let characters = INSTRUCTIONS.chars().count();

        assert!(
            characters < 5_500,
            "INSTRUCTIONS is {characters} characters; the budget is 5500"
        );
    }

    /// The `query` description is where a model learns what `evidence.kind`
    /// can hold, so it names every kind the schema's CHECK list allows.
    #[test]
    fn the_query_tool_description_names_every_evidence_kind_the_schema_allows() {
        let schema = include_str!("store/schema.sql");
        let check = schema
            .split("CHECK (kind IN (")
            .nth(1)
            .and_then(|rest| rest.split("))").next())
            .expect("the evidence kind CHECK list");
        let kinds: Vec<&str> = check
            .split(',')
            .map(|kind| kind.trim().trim_matches('\''))
            .collect();
        let surface = tool_surface();
        let query = surface
            .iter()
            .find(|tool| tool.name == "query")
            .expect("the query tool is listed");

        let description = query
            .description
            .as_deref()
            .expect("query has a description");

        assert_eq!(kinds.len(), 8, "{kinds:?}");
        for kind in kinds {
            assert!(
                description.contains(kind),
                "`{kind}` missing: {description}"
            );
        }
    }

    #[test]
    fn read_resource_reports_an_unknown_uri_as_resource_not_found() {
        let error =
            read_resource_contents("slugaudit://nope").expect_err("an unknown URI is not served");

        assert_eq!(
            error.code,
            rmcp::model::ErrorCode::RESOURCE_NOT_FOUND,
            "{error:?}"
        );
        assert_eq!(error.message, "unknown resource `slugaudit://nope`");
    }

    #[test]
    fn read_resource_refuses_a_uri_over_the_cap_without_echoing_it() {
        let uri = "u".repeat(MAX_RESOURCE_URI_BYTES + 1);

        let error = read_resource_contents(&uri).expect_err("an over-cap URI is refused");

        assert_eq!(
            error.code,
            rmcp::model::ErrorCode::INVALID_PARAMS,
            "{error:?}"
        );
        assert_eq!(error.message, "resource uri exceeds 8192 bytes");
        assert!(
            !error.message.contains("uuuu"),
            "an oversized URI must not be echoed into the message: {}",
            error.message
        );
    }
}
