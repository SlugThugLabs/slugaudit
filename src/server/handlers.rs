// slugaudit-line-exception: approved-by=split-judge; reason=Single declarative MCP #[tool_router] handler registry (input_schema helper, register_tools! forwarding macro, and 10-tool wire metadata/description table); length is advertised tool descriptions, not mixed concerns or duplication.
//! The `#[tool]` handler registry for [`SlugAuditServer`].
//!
//! Every handler is declarative: it names the tool and forwards its request
//! to `crate::tools` through `SlugAuditServer::dispatch` (or `dispatch_health`
//! for `health`). The titles, descriptions, and annotations here are the
//! advertised wire surface checked by `check_mcp_surface`.

use crate::server::SlugAuditServer;
use crate::tools;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::RequestMetaObject;
use rmcp::{ErrorData, Peer, RoleServer, tool, tool_router};
use std::sync::Arc;

/// The `inputSchema` of a tool whose arguments are `T`: the generated schema
/// without its top-level `title` and `description`.
pub(super) fn input_schema<T: schemars::JsonSchema + std::any::Any>() -> Arc<rmcp::model::JsonObject>
{
    let mut schema = rmcp::handler::server::common::schema_for_type::<T>()
        .as_ref()
        .clone();
    schema.remove("title");
    schema.remove("description");
    Arc::new(schema)
}

macro_rules! register_tools {
    (@call $self:ident, $fn_name:ident, $meta:ident, $peer:ident, $op:expr, dispatch) => {
        $self
            .dispatch($meta, $peer, stringify!($fn_name), $op)
            .await
    };
    (@call $self:ident, $fn_name:ident, $meta:ident, $peer:ident, $op:expr, dispatch_health) => {
        $self.dispatch_health($meta, $peer, $op).await
    };
    (
        $(
            $fn_name:ident ( $req:ty => $resp:ty ) via $dispatch:ident {
                title = $title:literal,
                read_only = $ro:literal,
                destructive = $destr:literal,
                description = $desc:literal $(,)?
            }
        ),* $(,)?
    ) => {
        #[tool_router(vis = "pub(crate)")]
        impl SlugAuditServer {
            $(
                #[tool(
                    input_schema = input_schema::<$req>(),
                    title = $title,
                    annotations(read_only_hint = $ro, destructive_hint = $destr),
                    description = $desc
                )]
                async fn $fn_name(
                    &self,
                    meta: RequestMetaObject,
                    peer: Peer<RoleServer>,
                    request: Parameters<$req>,
                ) -> Result<Json<$resp>, ErrorData> {
                    register_tools!(
                        @call self,
                        $fn_name,
                        meta,
                        peer,
                        move |sink, manager| {
                            tools::$fn_name(&request, sink.as_ref(), &manager)
                        },
                        $dispatch
                    )
                }
            )*
        }
    };
}

register_tools! {
    report(tools::ReportRequest => tools::ReportResponse) via dispatch {
        title = "Project snapshot",
        read_only = true,
        destructive = false,
        description = "Automatic project snapshot: file/language counts, parser failures, evidence-kind counts, open findings, and inbound/outbound dependency-edge counts. Use this first to understand the project's shape, then query for specifics. No score, no risk leads. `notices` flags untested languages (only Rust is tested); `languages[].tested` says which.",
    },
    leads(tools::LeadsRequest => tools::LeadsResponse) via dispatch {
        title = "Mechanical leads",
        read_only = true,
        destructive = false,
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
         whole surface. `notices` names any untested language the counts cover (only Rust is tested).",
    },
    mechanic(tools::MechanicRequest => tools::MechanicResponse) via dispatch {
        title = "Register auditor mechanics",
        read_only = false,
        destructive = false,
        description = "Register, remove, or list your own mechanics. The built-in `leads` \
         mechanics are counting primitives, not a checklist: what counts as a signal in this \
         codebase is your decision, and this is where you record it. `action: add` takes `id` \
         (a-z, 0-9, _; never a built-in's or an existing entry's id), `language`, `query` (a \
         Tree-sitter S-expression with exactly one capture; each distinct captured node is one \
         row) and `note` (what it counts, in your words); a query that does not compile or whose \
         trial scan ends `partial` is refused. It returns `first_count`: the `leads` response \
         for that id (counts with `covers`, the first page of rows, `page`, `files_total`, \
         `files_covered`, `files_not_parsed`). Entries are scoped to the project and live until \
         this server exits or `project_control` `off` runs. `leads` and `report` list every \
         entry with `source` (`session`, `builtin`) and its note, session first. `action: remove` \
         deletes a registered entry by `id`; built-ins can never be removed or overridden. \
         `action: list` returns every session and built-in mechanic, paged by cursor.",
    },
    query(tools::QueryRequest => tools::QueryResponse) via dispatch {
        title = "Read-only SQL query",
        read_only = true,
        destructive = false,
        description = "Query the indexed codebase with read-only SQL. Key tables: \
         files (id, path, language, byte_len, content — full text of every indexed file including non-code files), \
         evidence (id, file_id, kind, start_line, end_line [1-based, inclusive], payload; kind is one of Structure, Import, Export, Comment, Docstring, Symbol, Diagnostic, Call), \
         dependency_edges (from_file_id, to_file_id, raw_import_text, resolution_kind, confidence, syntax_unmodeled). \
         To inspect code or file content directly from the DB without filesystem reads, query `files.content` (e.g. `SELECT content FROM files WHERE path = 'src/main.rs'`); `line_count(text)` and `line_range(text, start, end)` read a file by 1-based lines, and an oversized first row is clipped with a `substr` continuation in `clipped_cells`. \
         To link evidence to file paths, always JOIN: `SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure'`. \
         Schema discovery: `SELECT name FROM sqlite_master WHERE type='table'`. \
         A statement SQLite cannot compile or run fails with code `sql_error`.",
    },
    structure(tools::StructureRequest => tools::StructureResponse) via dispatch {
        title = "AST structure query",
        read_only = true,
        destructive = false,
        description = "Inspect and extract code AST structures from a file or across the project using Tree-sitter. \
         Specify `file` to search a single file, or omit `file` and specify `language` (with optional `pattern`) \
         to search across matching project files (a `language` no indexed file has fails with \
         `unknown_language`, listing the languages indexed here). Returns matched AST nodes with `file`, `start_line`, `end_line` (1-based, inclusive), \
         and `text` snippet. The `query` must name at least one capture with `@name`. \
         Examples: Rust `(function_item name: (identifier) @name body: (block) @body)`, \
         Python `(function_definition name: (identifier) @name body: (block) @body)`, \
         TypeScript/JS `(function_declaration name: (identifier) @name)`, \
         or whole-node `(class_definition) @cls`. `notices` warns when the language is untested (only Rust is).",
    },
    finding(tools::FindingRequest => tools::FindingResponse) via dispatch {
        title = "Record a finding",
        read_only = false,
        destructive = false,
        description = "Persist a conclusion you have personally reviewed about a specific file \
         and line range. Supply `path` (any path in the active project, to select the database), \
         `file` (project-relative path of the file), `line_start`/`line_end` (one-based inclusive), \
         plus `severity`, `category`, `title`, and `description` (all your own judgment — SlugAudit \
         never generates these). The finding is bound to the file's current content hash and \
         `finding_read` reports it `stale` the moment that file changes. Findings last until \
         this server exits. Use this to record reviewed issues, not raw diagnostics.",
    },
    finding_read(tools::FindingReadRequest => tools::FindingReadResponse) via dispatch {
        title = "Read session findings",
        read_only = true,
        destructive = false,
        description = "Retrieve the findings recorded in this session (since this server \
         started); a restart begins with none. Each carries `status`: `current` while its file \
         still has the content hash it was written against, `stale` once that file changed or \
         is gone. Supply `path` (any path in the active project) to select the project, and \
         optionally `file` to filter to a specific file. Results are byte-cut pages under the \
         shared `page`; call again with only `page.cursor` for the next page.",
    },
    finding_clear(tools::FindingClearRequest => tools::FindingClearResponse) via dispatch {
        title = "Clear session findings",
        read_only = false,
        destructive = true,
        description = "Delete every finding recorded in this session for the project and report \
         how many were removed (`removed`). Supply `path` (any path in the active project). Use \
         it to start clean when the user asks, for example after switching models mid-session. \
         A restart of this server already begins with no findings.",
    },
    project_control(tools::ProjectControlRequest => tools::ProjectControlResponse) via dispatch {
        title = "Enable or disable a project",
        read_only = false,
        destructive = true,
        description = "Enable or disable SlugAudit for a project. Pass `action` = `\"on\"` to enable \
         a project (creates the directory `.planning/slugaudit/`, which is the on switch, builds the database and \
         runs the initial import), or `\"off\"` to disable it (deletes the whole `.planning/slugaudit/` \
         directory, then `.planning` if it is empty). A project is on only while that \
         directory exists; if its database is missing or broken, the next tool call rebuilds it. On `\"on\"` the \
         runtime directory `.planning/slugaudit/` is also appended to the project-root `.gitignore` \
         unless a rule that already ignores it is present; the response's `gitignore` field reports \
         `added`, `already_ignored`, or `not_written` plus the file's path. `\"off\"` never edits \
         `.gitignore`. Supply `path` to \
         target a specific project root; defaults to the current directory. If the project was \
         moved, copied, or re-extracted elsewhere (e.g. unzipped fresh from an archive), a stale \
         `.planning/slugaudit/project.db` is discarded and rebuilt from source automatically — the \
         agent does not need to delete it or re-enable manually. `profile` (`lean`, `audit` or \
         `adaptive`) sets the resource profile for this session only; it is not saved.",
    },
    health(tools::HealthRequest => tools::HealthResponse) via dispatch_health {
        title = "Operational health",
        read_only = true,
        destructive = false,
        description = "What state the server is in and what to do next: `phase`, `remedy` (the \
         call to make when no project is served), and `database.revision_id` / \
         `database.file_count`. Read-only: never syncs, never mutates, and answers a path that \
         is not an enabled project with `phase: disabled` instead of an error. Pass `path` for \
         a project's database state; without it no project is selected. `verbose: true` adds \
         `detail`, the operator view (parser pack version, skipped files, process counters, \
         host memory, result store).",
    },
}
