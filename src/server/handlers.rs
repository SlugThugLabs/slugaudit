//! The `#[tool]` handler registry for [`SlugAuditServer`].
//!
//! Every handler is declarative: it names the tool and forwards its request
//! to `crate::tools` through `SlugAuditServer::dispatch`, which owns the
//! progress sink and the blocking-pool semaphore. The titles, descriptions,
//! and annotations here are the advertised wire surface — `check_mcp_surface`
//! snapshots them beside `resources::list()` — so they are kept byte-identical
//! to the pre-split file. `#[tool_router]` also generates the
//! `SlugAuditServer::tool_router()` constructor the server struct stores.
//!
//! The router constructor is `pub(crate)` because it is built in this child
//! module and called from `server.rs`, the parent that owns the field.

use crate::server::SlugAuditServer;
use crate::tools;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::RequestMetaObject;
use rmcp::{ErrorData, Peer, RoleServer, tool, tool_router};

#[tool_router(vis = "pub(crate)")]
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
         with its row count, its distinct-file count (`files`) and `covers` (the languages that \
         count read; a language missing there is unchecked, not absent), then call again with one `mechanic` id to get \
         file/line rows, optionally narrowed with `pattern` (a path GLOB, e.g. `src/auth/*`; \
         without `*`/`?` it is a substring match). Every response carries the shared `page` \
         (`returned`, `total`, `complete`, `cursor`, `page_bytes`, `bytes_used`, `limited_by`, \
         `revision_id`) plus the file denominators `files_total`/`files_covered`; there is no row \
         cap, so a page is cut only by your `limit` (omit it for everything that fits) and by the \
         page byte size (64 KiB by default; `page_bytes` raises it up to the server ceiling). To \
         page, call again with only `page.cursor`: the pages are one stored answer from the \
         first call, so edits made meanwhile never skip or repeat rows. For `mechanic`/`file`/`line` triples \
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
        title = "Register auditor mechanics",
        annotations(read_only_hint = false, destructive_hint = false),
        description = "Register, remove, or list your own mechanics. The built-in `leads` \
         mechanics are counting primitives, not a checklist: what counts as a signal in this \
         codebase is your decision, and this is where you record it. `action: add` takes `id` \
         (a-z, 0-9, _; never a built-in's or an existing entry's id), `language`, `query` (a \
         Tree-sitter S-expression with exactly one capture; each distinct captured node is one \
         row) and `note` (what it counts, in your words); a query that does not compile is \
         refused with the compiler's message. It returns `first_count`: the `leads` response \
         for that id (counts with `covers`, the first page of rows, `page`, `files_total`, \
         `files_covered`, `files_not_parsed`). Session entries live until this server exits; \
         `persist: true` writes the entry to `.planning/slugaudit/config.json` (other keys \
         kept) so later sessions read it. `leads` and `report` list every entry with `source` \
         (`session`, `project_config`, `builtin`) and its note, session first. `action: remove` \
         deletes a session or persisted entry; built-ins can never be removed or overridden. \
         `action: list` returns every mechanic plus `invalid`: persisted entries that failed to \
         load, each with its error."
    )]
    async fn mechanic(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::MechanicRequest>,
    ) -> Result<Json<tools::MechanicResponse>, ErrorData> {
        self.dispatch(meta, peer, "mechanic", move |sink, manager| {
            tools::mechanic(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        title = "Read-only SQL query",
        annotations(read_only_hint = true, destructive_hint = false),
        description = "Query the indexed codebase with read-only SQL. Key tables: \
         files (id, path, language, byte_len, content — full text of every indexed file including non-code files), \
         evidence (id, file_id, kind, start_line, end_line [1-based, inclusive], payload; kind is one of Structure, Import, Export, Comment, Docstring, Symbol, Diagnostic, Call), \
         dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled). \
         To inspect code or file content directly from the DB without filesystem reads, query `files.content` (e.g. `SELECT content FROM files WHERE path = 'src/main.rs'`); `line_count(text)` and `line_range(text, start, end)` read a file by 1-based lines, and an oversized first row is clipped with a `substr` continuation in `clipped_cells`. \
         To link evidence to file paths, always JOIN: `SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure'`. \
         Schema discovery: `SELECT name FROM sqlite_master WHERE type='table'`. \
         A statement SQLite cannot compile or run fails with code `sql_error`."
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
         to search across matching project files (a `language` no indexed file has fails with \
         `unknown_language`, listing the languages indexed here). Returns matched AST nodes with `file`, `start_line`, `end_line` (1-based, inclusive), \
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
         optionally `file` to filter to a specific file. Results are byte-cut pages under the \
         shared `page`; call again with only `page.cursor` for the next page."
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
        title = "List or adopt earlier sessions' findings",
        annotations(read_only_hint = false, destructive_hint = false),
        description = "A restart starts a new session: `finding_read` shows only this session's \
         findings, and earlier sessions' findings are kept but marked stale. Supply `path` (any \
         path in the active project) and `action`. `list` returns earlier sessions with \
         `session_id`, `started_at_unix` (their first finding) and `findings`. `adopt` with a \
         `session_id` from `list` makes that session's findings yours, so `finding_read` returns \
         them; each reads `current` if its file is unchanged, else `stale`. Adopt only when you \
         are continuing that audit; nothing is ever deleted."
    )]
    async fn finding_sessions(
        &self,
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
        request: Parameters<tools::FindingSessionsRequest>,
    ) -> Result<Json<tools::FindingSessionsResponse>, ErrorData> {
        self.dispatch(meta, peer, "finding_sessions", move |sink, manager| {
            tools::finding_sessions(&request, sink.as_ref(), &manager)
        })
        .await
    }

    #[tool(
        title = "Enable or disable a project",
        annotations(read_only_hint = false, destructive_hint = true),
        description = "Enable or disable SlugAudit for a project. Pass `action` = `\"on\"` to enable \
         a project (creates the activation directory and database and runs the initial import), or `\"off\"` to \
         disable it (removes the database, its sidecars, the results directory and the config lock; \
         `config.json` is kept and `on` restores its settings). On `\"on\"` the \
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
        description = "What state the server is in and what to do next: `phase`, `remedy` (the \
         call to make when no project is served), `watcher.health`, and `database.revision_id` / \
         `database.file_count`. Read-only: never syncs, never mutates, and answers a path that \
         is not an enabled project with `phase: disabled` instead of an error. Pass `path` for \
         a project's database state; without it only the registered project's watcher is \
         reported. `verbose: true` adds `detail`, the operator view (unreconciled watcher \
         counts, parser pack version, skipped files, process counters, host memory, the \
         mechanic scan cache)."
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
