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
    PAGING: `leads`, `query`, `structure` and `finding_read` return one `page` object: `returned`, `total` \
    (null when not counted), `complete`, `next_offset` (present only while more remain), `page_bytes` (the byte \
    budget after the clamp to the server ceiling), `bytes_used`, `limited_by` (`none`, `limit`, `page_bytes`, \
    `ceiling` — a larger `page_bytes` returns nothing more —, `match_cap`, `scan_window`, `time_budget`) and \
    `revision_id`. Pages are cut by bytes; the only count bounds are your own `limit` (`leads`) and \
    `structure`'s per-page match cap. To page, pass `offset` = `page.next_offset` and \
    `expected_revision` = `page.revision_id`: if the index changed mid-sweep the call is refused with \
    `stale_revision` and a remedy restarting from offset 0, instead of silently skipping or repeating rows. \
    `structure` alone has a second cursor beside `page`: when `complete` is false and `next_offset` is absent, \
    repeat with `start_after_file` = `next_start_after_file` and `offset` 0.\n\n\
    LEADS: `leads` with only `path` returns every mechanic id with its row count, its distinct-file count \
    (`files`), and `covers` — the sorted languages of the files under `pattern` that mechanic read, the \
    machine-readable form of \"a zero here means unchecked, not absent\" — your registered mechanics first (see MECHANICS), then the built-ins in declaration order: call sites by callee (`unwrap`, `expect`, \
    `panic`/`unreachable`, `todo`/`unimplemented`, `assert`/`assert_eq`/`assert_ne`/`debug_assert*`), \
    diagnostics, TODO/FIXME/HACK/XXX comment markers, and credential-shaped file paths. Counts are complete \
    regardless of `limit`/`offset` and carry no ordering by importance. Call again with one `mechanic` id for \
    file/line rows, optionally narrowed with `pattern` (a path GLOB like `src/auth/*`; without `*` or `?` it is \
    a substring match) — it filters the counts and the rows alike. Pass `mode: compact` to drop each row's \
    `detail` and get `mechanic`/`file`/`line` triples instead; with no `mechanic` that enumerates the whole \
    surface in one response, under the same paging fields. Every response carries `page` (see PAGING), \
    plus the file denominators `files_total`/`files_covered`: at scale the \
    number you can actually exhaust is the files containing the mechanic, not the rows. There is no row cap — a \
    page is cut only by your `limit` (omit it for everything that fits) and by the page byte size \
    (64 KiB by default; pass `page_bytes` for more, up to the server ceiling); `mechanic_counts` and \
    `page.total` are the complete size up front, and a first row bigger than the page is still returned and \
    flagged `oversized_row`. Three limits to know. (1) The callee-based mechanics \
    read Call evidence, extracted for rust, python, javascript, typescript, tsx, go, java, and c only; their \
    callee lists are Rust names, so in another language they count only the calls that share those names. A \
    language absent from `covers` was not checked: its zero means nothing was read, not that nothing is there \
    — diagnostics and comment markers read every parsed file, credential paths every file. (2) Each row's `detail` is clipped to 200 characters, and \
    `detail_clipped` is true when characters were dropped; the remainder lives only in the file. (3) Every \
    response carries `files_not_parsed`: indexed files under your `pattern` with a detected language whose \
    grammar did not run or finish (content over \
    `max_parse_bytes`, 2 MiB by default; a grammar that failed to load; a failed or timed-out parse); while it \
    is above 0 every count is a floor, and `report` lists those files with the reason.\n\n\
    MECHANICS: the built-ins are counting primitives, not a checklist; what counts as a signal in this codebase is \
    your decision. `mechanic` with `action: add` registers `id`, `language`, `query` (a Tree-sitter S-expression \
    with exactly one capture; each distinct captured node is one row; one that does not compile is refused with \
    the compiler's message) and `note`, and returns `first_count`: the `leads` response for that id. `leads` then \
    counts and pages it like a built-in, and `leads` and `report` list every entry with `source` (`session`, \
    `project_config`, `builtin`) and its note, session first. Session entries last until this server exits; \
    `persist: true` writes the entry to `.planning/slugaudit/config.json` for later sessions. A built-in can never \
    be removed or overridden. `report.mechanics.invalid` lists persisted entries that failed to load, with the \
    error and `index`; `mechanic remove` with that `index` deletes one no `id` can name.\n\n\
    QUERY SCHEMA: `files` (id, path, language, byte_len, content — full text of every indexed file, including \
    non-code files), `evidence` (id, file_id, kind, start_line, end_line [1-based, inclusive; every line number in this server is 1-based], payload — JSON; kinds Structure, \
    Import, Export, Comment, Docstring, Symbol, Diagnostic, Call), `dependency_edges` \
    (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled). One read-only \
    SQL statement per call, with no row cap, cut only by the page byte size: 64 KiB by default, `page_bytes` \
    overrides it up to the server ceiling (see PAGING). `page.total` is the exact size of the whole result, so \
    you see how large it is before you read it (if counting ran out of budget it is null and `total_unavailable` \
    says why). Add `LIMIT` to exploratory queries, page with `page.next_offset`, or narrow the query: some MCP clients pass a large tool \
    result to the model unbounded, which fails the turn. A first row bigger than the page (one huge \
    `files.content`, a `group_concat`) is not returned whole: its TEXT/BLOB cells are clipped to fit and each cut is \
    listed in `clipped_cells` with `row`, `column`, `returned` and `total` (characters for text, bytes for a blob: \
    `substr`'s own units) and `continue_with`, a ready `substr(\"<column>\", start, len)` for the rest; nothing is lost, \
    and a larger `page_bytes` avoids the cut. JSON has no infinity, so an infinite REAL is `null` in `rows` and \
    listed in `non_finite_cells` (`row`, `column`, `value`). To read a file by lines, `line_count(text)` and \
    `line_range(text, start, end)` (1-based, inclusive, source line endings kept, NULL in gives NULL): SELECT \
    line_count(content), line_range(content, 400, 600) FROM files WHERE path = 'src/big.rs'. Always JOIN evidence \
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
    GRAMMARS: Tree-sitter grammars are not bundled; the first time a language is parsed, its grammar is fetched \
    from the language pack's GitHub release (sha256-verified, cached under \
    `~/.cache/tree-sitter-language-pack/`). Offline, files in that language are still stored and searchable by \
    `content`, but have no parsed evidence: their `files.parser_availability` is `LoadFailed`, and they are \
    retried after a 10-minute cool-down or on the next server start. The first failed fetch can block one call \
    for up to about 60 s. Markdown, reStructuredText, AsciiDoc, Org and plain-text files, and the ignore lists `.gitignore`, `.ignore` and `.dockerignore`, are stored as \
    `language = 'prose'` and never parsed (no grammar, no evidence; `structure` refuses them).\n\n\
    STRUCTURE: `structure` runs a Tree-sitter S-expression query (required, and it must name at least one \
    capture) against indexed content — in one `file`, or project-wide with `language` and an optional `pattern` \
    path filter (e.g. `src/**/*.rs`). Rust `(function_item name: (identifier) @name)`, \
    Python `(function_definition name: (identifier) @name)`. Results page at 64 KiB under `page` (see PAGING; `page_bytes` overrides); `files_not_scanned` > 0 means the project-wide scan hit its memory window, and the second cursor `next_start_after_file` (with `offset` 0) resumes it once `page.next_offset` is exhausted; `unparsed_files` lists files that yielded no tree. Ready-made recipes are resources: \
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
    FLOW: report → leads → mechanic add for the shapes you decide matter → drill with query/structure → read \
    source only where evidence points → finding.\n\n\
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
