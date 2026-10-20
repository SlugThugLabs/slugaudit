# SlugAudit Architecture

> **Product boundary:** SlugAudit is shipped to users as the single
> `slugaudit` binary. It runs inside a user's project through an
> MCP-compatible agent and stores disposable derived data in
> `<project-root>/.planning/slugaudit/project.db`. In this repository,
> `.planning/` contains SlugAudit's development documentation; in a customer
> project, `.planning/` is customer-owned project data. SlugAudit owns only the
> `slugaudit/` child directory and excludes it from discovery.

This document describes what the system *is*: its structure, its data flow,
its invariants, and the reasoning behind the choices that shape them. It is
written so an engineer can predict where a change belongs and which
constraints a change must not break. It is not a plan (`.planning/PLAN.md`), not an
operator runbook (`docs/TROUBLESHOOTING.md`), and not a guide to wiring up a
specific client (`docs/CLIENTS.md`).

## What SlugAudit is

SlugAudit is a **stdio MCP server** (`slugaudit serve`) backed by a
per-project SQLite database. It parses a project's source into evidence —
structure, symbols, imports, call sites, comments, diagnostics — and answers
queries about that evidence. It is an index the calling agent queries instead
of re-reading files.

It is **not an auditor and never will be**: it performs no risk detection,
assigns no severity, ranks nothing, and draws no conclusions. Every judgment
in a response belongs to the calling agent. This is a structural property, not
a tone-of-voice choice:

- the schema has no table whose purpose is to claim SlugAudit found a bug
  (`src/store/schema.sql`);
- the `findings` table holds text the agent supplied, verbatim
  (`src/tools/finding.rs`);
- `leads` returns unranked counts and rows derived at read time
  (`src/tools/leads.rs`).

The server exposes eight tools (`src/server.rs`):

- **`report`** — automatic snapshot of the current revision: file/language
  counts, parser failures, evidence-kind counts, import-resolution buckets,
  inbound/outbound `dependency_edges` totals, credential-shaped file count.
- **`leads`** — unranked mechanical observations: call sites by callee name,
  diagnostics, TODO/FIXME/HACK comment markers, credential-shaped files.
  Counts by default, each with its row count, distinct-file count and
  `covers` (`rust_only` / `all_languages`); file/line rows for one mechanic on
  request, optionally narrowed by a `pattern` path filter. Rows are cut by a
  response byte budget rather than a row cap, and every rows response reports
  `returned` / `total` / `complete` with `files_total` / `files_covered`. A
  resumed page may pin `expected_revision` and is refused (`stale_revision`)
  when the index moved on. Callee-based mechanics are Rust-only in this
  revision.
- **`query`** — arbitrary read-only SQL against the project's own database.
  The connection itself refuses writes; the SQL is never inspected for write
  intent.
- **`structure`** — Tree-sitter structural pattern matching against indexed
  source content, either single-file or project-wide.
- **`finding`** — the single write tool. Persists agent-authored conclusions
  bound to a file's current `content_hash`.
- **`finding_read`** — returns findings scoped to the calling agent session.
- **`project_control`** — enable or disable a project (create or remove the
  activation directory and run the initial import).
- **`health`** — what state is this in and what should the caller do next.
  Answers with three groups, each present or `null` as a unit so absence
  reads as absence: `watcher` (health, unreconciled counts, sequences),
  `database` (revision id, parser pack version, file and skipped-file
  counts), and `server` (tool-call counters, last-sync timestamp and
  duration, consecutive full publishes). `phase` and `remedy` summarize the
  first two; `system_memory` reports host state because it sizes the
  adaptive import budget.

Only `finding` and `project_control` mutate state. Every state-bearing tool
first brings the project current (see *Data flow*); if the watcher reports
untrusted state, that sync is a full publish rather than a trust of the
dirty-set.

The same server exposes read-only MCP resources (`src/resources.rs`): the
canonical schema (`slugaudit://schema.sql`), the query recipes under
`slugaudit://queries/` (`examples/queries/`), and
`slugaudit://queries/evidence_payload_keys.md` — the per-kind
`evidence.payload` key/type reference that would otherwise sit in every
session's `INSTRUCTIONS`. They are embedded at compile time via `include_str!`
and are pull-based — nothing is pushed into a connecting agent's context.

Every tool also carries a `title`, MCP `annotations` (`readOnlyHint` false for
`finding`/`project_control`, `destructiveHint` true only for
`project_control`, whose `off` action purges the database), and an
`outputSchema` derived from its response type. `check_mcp_surface` snapshots
`tools/list` + `resources/list` into `.planning/archive/mcp_surface.json` and
fails on drift, so none of those fields can change without a recorded,
reviewable update.

## Repository versus runtime scope

`.planning/` at this repository root is development material for SlugAudit; it
is not shipped in the binary. When the binary runs in a customer project, that
project's `.planning/` directory is customer data and may be indexed. Only
`<project-root>/.planning/slugaudit/` is application-owned runtime state, and
its `project.db` is disposable derived data. `src/ignore_rules.rs` is the
single source of truth for what is excluded (the activation directory, VCS
internals, scratch files, and the project's own ignore files), consulted by
discovery, the watcher, and reconcile alike so those three can never disagree
about what the index contains.

## Layering

Bottom-up: `store` and `parse` know nothing of MCP. `graph` knows nothing of
`sync`. `sync` knows nothing of `tools` — including `progress`, which exists
precisely so the sync layer can emit progress events without depending on the
MCP-aware code that renders them. `tools` orchestrates across the others but
never duplicates their logic.

Three single-entry-point rules keep the graph acyclic and the write surface
narrow:

- `sync::revision::publish_revision` is the only path that writes `files`,
  `evidence`, `dependency_edges`, and `revisions`.
- `tools::context::ensure_synced` is the only path that brings a project
  current. Every state-bearing tool calls it — `report` included.
- `tools::ToolCounters` is the only definition of "how many tool calls have we
  served". A parallel counter elsewhere would be a second source of truth.

## Subsystems and module layout

SlugAudit is organized into cohesive subsystems under `src/`. For the exhaustive file-by-file manifest, see [.planning/MODULE_MAP.md](MODULE_MAP.md).

- **`tools/`** — MCP tool handlers and orchestration (`report`, `query`, `structure`, `leads`, `finding`, `finding_read`, `project_control`, `health`). Thin boundary contracts delegating to verified storage transactions.
- **`sync/`** — Indexing engine: discovery, sampling, hashing, reconciliation, publish (compare-and-swap), revisions, and stat sweep.
- **`store/`** — SQLite persistence boundary: connection configuration, schema migrations, read-only and read-write connection guards.
- **`graph/`** — Dependency and import resolution across multiple language ecosystems (Rust, Python, JS/TS, C#, etc.).
- **`evidence/`** — Evidence normalization from parse trees into indexed structural facts.
- **`parse/`** — Tree-sitter language pack boundary and language detection.
- **`model/`** — Core shared domain contracts: source spans, limits, profiles, and evidence kinds.
- **`watch/`** — Filesystem watching and event notification debouncing.
- **`project/`** — Project root resolution, activation markers, and project configuration.
- **`bin/`** — Development-only gate binaries enforcing source limits, drift checks, surface checks, and governance invariants.

```
src/
├── main.rs                   binary entry: parse args, then serve MCP over stdio
├── lib.rs                    crate root: module declarations + crate-wide lint policy
├── tools/                    MCP tool handlers — thin orchestration
├── sync/                     detection and persistence of what changed on disk
├── graph/                    import evidence → dependency_edges
├── evidence/                 evidence extraction + normalization
├── watch/                    filesystem watching
├── project/                  project resolution + activation
├── store/                    SQLite connection boundary
├── parse/                    tree-sitter language-pack boundary
├── model/                    contracts shared by parsing, storage, and tools
└── bin/                      development-only gate binaries
```

The exhaustive source-file tree is maintained in [.planning/MODULE_MAP.md](MODULE_MAP.md) and verified by `src/bin/check_docs_drift/`. Adding or removing internal files updates the module map without requiring modifications to this architecture document.

## Data flow (one tool call)

```
Caller                             SlugAudit server
└─ MCP request                     run_blocking (semaphore-bounded)
   └─ tools::ensure_synced         → SourceSyncManager::ensure_current
      │                              <!-- mcp://progress: ensuring_current -->
      ├─ project::resolve_project    walk parents for `.planning/slugaudit`
      ├─ store::open_read_write      symlink/NFS/permission guards; WAL; schema
      ├─ ensure_project_row          INSERT OR IGNORE the singleton row; purge prior-session findings
      ├─ watch_manager.watch         register or reuse the project's WatchState
      └─ sync_by_health branch
         ├─ NeedsVerification│Desynced → full publish + drain events → Healthy|Partial
         │                              <!-- mcp://progress: publishing {i}/{total} -->
         ├─ Healthy│Partial → stat sweep, then reconcile unreconciled events
         │            (barrier-bounded to MAX_BARRIER_LOOPS), then apply sweep findings
         └─ Unavailable → full publish every call
   └─ tools::* handler             with_verified_read / with_verified_write
   └─ ToolCounters::record_call    bump call_count / error_count / total_ms
   └─ tracing::info!|warn!         one completion line: session_id, call_id, tool,
                                   elapsed_ms, outcome, ToolCounters deltas
   └─ Json<{…}>                    return
                                       <!-- mcp://progress: completed -->
Caller receives JSON response
```

The `<!-- mcp://progress: … -->` annotations mark the wire points where MCP
`/notifications/progress` events are emitted by `server_runner::run_blocking`:
`0.0` with message `{tool} ensuring_current` before the semaphore acquire,
`0.5` with `{tool} publishing` once the permit is held (per-file `Sampling`
events from the sync layer's `McpProgressSink` then overwrite this with the
real i/N ratio), and `1.0` with `{tool} completed` when the work finishes,
whether it succeeded or failed. Notifications are best-effort: a broken
progress channel can never fail a successful tool call. Phases the sync layer
reports through its own `ProgressEvent` stream (e.g. a reconcile stage between
publishing and completed) appear as labels of the same shape.

### The tool-call log

`run_blocking` wraps every call in one `tool_call` span carrying `tool`,
`call_id` (a process-local monotonic counter; `server_runner.rs`) and
`session_id` (`util::SESSION_ID`), so **every** line a call emits — the start
line, the `tools::*` lines inside it, and its completion line — shares one
correlation pair that distinguishes it from the next call to the same tool.
Each call emits exactly one completion event, the message `"tool call
completed"` at INFO on success and WARN on failure, carrying `outcome`
(`"success"` / `"error"`), `elapsed_ms`, and the process-wide `ToolCounters`
deltas observed across the call. The counters cannot be reset and concurrent
calls bump them too, so they are logged as deltas — never as absolutes or as
this call's exclusive share; a delta can cover a sibling call running at the
same time. Nothing else is logged: no tool arguments, no results (see the
redaction contract in `tools/redaction_tests.rs`).

A session's call sequence is recoverable by grepping one session id. With
`SLUGAUDIT_LOG_FORMAT=json`:

```
grep -F "\"session_id\":\"<uuid>\"" audit.jsonl \
  | jq -r 'select(.fields.message == "tool call completed")
           | [.span.call_id, .span.tool, .fields.outcome, .fields.elapsed_ms] | @tsv'
```

With the default text format the span prefix carries the same fields
(`tool_call{tool="report" call_id=1 session_id=<uuid>}: …`):

```
grep -F "session_id=<uuid>" audit.log | grep -F 'tool call completed'
```

## Publish and revision model

`ensure_current` (`sync/manager_orchestration.rs`) resolves the project,
opens the database (recovering or rebuilding it when necessary), verifies the
metadata row, and dispatches on watcher health (`sync/manager_health.rs`):

| Health | Action |
|---|---|
| `NeedsVerification`, `Desynced` | full publish, then drain events that arrived during it, then mark `Healthy` (or `Partial` when the watch scope is limited) |
| `Healthy`, `Partial` | stat sweep; reconcile unreconciled events if any; apply the sweep's findings |
| `Unavailable` | full publish every call |

`Partial` means the watcher is active but its scope walk stopped at a documented
`max_watch_entries` / `max_watch_depth` limit; directories beyond the limit are
unwatched, and the per-call stat sweep remains the freshness guarantee for them.
The walk and the `IgnoreRules` build run outside the manager lock; only the
watch-set diff and `watcher.watch`/`unwatch` are serialized. A scope-refresh
generation makes an ignore-file event that arrives during the walk force another
pass instead of being swallowed. These caps are process/env limits
(`SLUGAUDIT_MAX_WATCH_ENTRIES`/`_DEPTH`); the watcher is registered before a
project config is loaded, so `config.json` does not override them.

A publish is compare-and-swap. `sync/publish_cas.rs::publish` discovers, hashes,
and diffs the current disk state against what the database holds, then asks
`revision::publish_revision` to commit exactly that delta in one transaction.
Two hazards are retried the same way, up to `MAX_CAS_RETRIES = 4`, each retry
re-sampling from scratch: another publisher committing first (the CAS on the
current revision id), and a file changing on disk after this attempt sampled
it but before the transaction that would record it. Neither is papered over —
both fail closed into a retry rather than publishing a revision that does not
match a stable disk state. One `Deadline` spans every retry, so a pathological
repository cannot restart the clock and stall the tool call.

A disk state identical to the current revision skips publishing entirely: an
unchanged sync writes no rows, so the database does not grow when nothing
changed.

Within the transaction, `publish_revision` upserts added/modified files and
their evidence, deletes removed files (cascading their evidence and edges),
and flips `revisions.is_current` only as the transaction's last statements;
nothing is visible to another connection until `commit` succeeds. `revision_id`
is derived from the row id (`rev-{id}`), so it changes on every publish.

A `files` row means "this file is in the project", not "this file was read".
Every discovered file gets one: a file discovery or sampling skipped (oversize,
unreadable) is written as `file_kind = 'skipped'` with a `skip_reason` code
(`oversize` / `unreadable` / `stat_failed`), `content` NULL, `extraction_completeness =
'Unavailable'` and `parse_outcome = 'NotAttempted'` — so `count(*) FROM files`
counts it instead of losing it, and `report.completeness` states how many rows
are skipped and why. Its identity is stat-derived (`hash::stat_identity`:
BLAKE3 over path + size + mtime, tagged `stat-identity`, never `blake3-v1`),
because reading the file to hash it would cost exactly what skipping avoids;
the accepted consequence is the stat sweep's own documented blind spot — a
skipped file edited to the same size within the same mtime second is not
detected. A path with no representable text form (a non-UTF-8 name) still has
no row and is visible only in the publish report and the server log. A
skipped file whose `stat` fails (deleted mid-sync, permissions changed, broken
symlink) is recorded with `skip_reason = 'stat_failed'` and `files.byte_len`
NULL: NULL means the size is unknown, and only a real empty file stores 0. A
sum over `byte_len` ignores unknown sizes, so pair it with
`count(*) WHERE byte_len IS NULL`.

Every row also carries `file_class` — `authored`, `generated`, or `vendored` —
derived from its path alone by `sync::classify::classify`, never from content,
so it is available for a skipped row too. It is a fixed fact, not a judgment:
a mechanic whose rows are dominated by `vendor/` can be summarised in one
sentence, and `report.completeness.file_class_counts` is the breakdown. The
schema column comment documents the heuristic and `vendored`'s precedence over
`generated`.

Existence rows take part in the diff and the manifest hash, which is what keeps
a skipped file from being classified as deleted (it is on disk) and keeps an
unchanged tree hashing to the same value. `known_paths` still unions the
caller's walked set with the stored rows: the walked set is the transaction's
authoritative statement of what exists, and the union costs one set operation.

A successful `ensure_current` never guarantees that no concurrent publish will
follow. `tools/context_transactions.rs::with_verified_read` and
`with_verified_write` therefore open a fresh connection, begin a transaction,
and re-verify the expected revision id inside it; a mismatch is retried by the
caller. That is the completeness check between "the revision we synced to" and
"the revision we are reading from".

## Dependency-edge resolution scope

`graph/` resolves each captured `Import` evidence item into exactly one
`dependency_edges` row. Resolution is syntactic and file-set-local: no
language's real module resolver is run, `node_modules` and package indexes are
never consulted, and cross-language edges are never inferred. Every row keeps
the verbatim `raw_import_text` whatever the outcome — the column is `NOT NULL`,
and an import with no reference at all is still recorded, as `Unresolved`.

| Language | Resolves to a project file | Otherwise |
|---|---|---|
| Python | dot-prefixed relative imports (`from . import x`, `from ..pkg import y`), including the `__init__.py` package marker | a single-token absolute import (`import os`, `from collections import X`) is `External`; a dotted absolute path (`from foo.bar import X`) is probed as a module path and resolves when the file exists, else `Unresolved` |
| Rust | `crate::` (anchored at the owning crate's `src`, found by walking up to the nearest indexed `Cargo.toml`), plus `super::`/`self::` | `std::` and bare crate names are `External` |
| JavaScript / TypeScript | relative `./x` / `../x`, probing the configured extensions and a directory `index` file | bare package names are `External`; `node_modules` is never searched |
| every other language | the generic fallback, which models relative paths, module paths, and the `import`/`from`/`using`/`use`/`open`/`#include`/quoted forms | a bare name is `External`; an import whose syntax the fallback does not model is `Unresolved` |

Confidence rides only on `Resolved` edges: exactly one candidate matches the
known file set is `High`, more than one is `Low` (the first candidate is
reported rather than discarded), and no match is `Unresolved` with no
confidence. Rust `super::`/`self::` are always `Low` because `mod`
declarations are never read, so which module a path names cannot be proven
from the file tree alone.

`syntax_unmodeled = 1` on an `Unresolved` edge marks syntax the resolver could
not parse at all (e.g. Go's `import "fmt"`), as opposed to syntax that parsed
but matched no project file (e.g. `kotlin.math.max`). The flag is computed
once per edge at publish time and `report` returns the split as
`import_resolution` plus `unsupported_language_unresolved_count`.

The consequence for a caller is direct: on many projects the edge table is
mostly `External`/`Unresolved`, and a graph in that shape makes dependency
walking a dead end rather than a finding.

## Freshness model

Freshness has two layers, and the ordering matters:

- **The `notify` watcher is an optimization.** Its callback runs on `notify`'s
  internal event loop and takes the manager state with `try_lock`; if a tool
  thread holds the lock, the event is dropped rather than blocking the event
  loop. A dropped event is harmless because the next call re-verifies. The
  watcher's error arm marks every project `Desynced`, which forces a full
  publish on the next call.
- **The stat sweep is the guarantee** (`sync/sweep.rs`). On every `Healthy`
  or `Partial` call the project is walked with the same walker discovery uses, and every
  file is `stat`ed — one `stat`, no read. A file whose stored
  `modified_unix_seconds` and `byte_len` both match is proven unchanged;
  everything else (stat mismatch, new on disk, or present in the database but
  gone from disk) becomes a reconcile candidate or a deletion. Candidates are
  only nominees: the reconcile pipeline re-hashes each one and skips files
  whose content hash still matches, so a false positive costs one hash, never
  a parse.

    A failed sweep fails closed: it sets the watcher `Desynced` and the call
    errors, because a partial sweep that silently skipped the rest of the tree
    would serve exactly the staleness the sweep exists to prevent.

**Accepted blind spot.** A same-size in-place edit that lands in the same
wall-clock second as the stored stat *and* produces no watcher event is not
detected by the sweep. It is covered whenever either layer fires (the watcher
event, or a later second), and the residual case is accepted rather than
hidden.

The sweep reconciles against the same ignore rules a full publish uses, and
`WatchManager::refresh_scope` recomputes the watch set when an ignore file
changes, so the incremental path indexes exactly what a fresh publish would
index — including files that became ignored, which converge out of the index
as deletions.

## Concurrency and deadlines

- **Blocking work is bounded, not the async runtime.** Every tool call's
  filesystem/SQLite/parse work goes through
  `server_runner::run_blocking`, which acquires a permit from one
  `Arc<Semaphore>` of `MAX_CONCURRENT_BLOCKING_OPS = 8` per server before
  moving the closure onto Tokio's blocking pool. The permit is moved *into*
  the closure, so a cancelled caller cannot free it while the blocking work
  runs. The acquire is an async yield, not a thread block.
- **Every operation has its own deadline.** Sync checks its deadline
  cooperatively (per discovered file, per dirty path, per barrier iteration).
  `query` has an SQLite VM-step budget and a wall-clock budget enforced
  through the progress handler. `structure` has a wall-clock budget enforced
  through Tree-sitter's native progress callback. The permit is bounded
  because the work it guards is bounded.
- **Barrier sync loops at most `MAX_BARRIER_LOOPS = 16` times**
  (`sync/reconcile/mod.rs`). Snapshot dirty/deleted sets, reconcile, then loop
  if new events arrived during reconciliation. Exceeding the cap means a
  pathological producer is racing reconciliation; the watcher is marked
  `Desynced` and `ReconcileError::BarrierCapExceeded` is returned rather than
  draining forever.
- **State is per server, not global.** `SlugAuditServer` owns its
  `SourceSyncManager` (and thus its watcher, ignore rules, and last-sync
  counters) and its semaphore, and passes the manager down explicitly. Two
  servers in one process track independent state. The process-wide state that
  remains is deliberate and small: `ToolCounters` atomics, `util::SESSION_ID`,
  `model::limits::process_limits` (a `OnceLock`), and `parse::PACK_VERSION`.
- **Every `Mutex` lock goes through `util::lock_or_recover`.** A mutex
  poisoned by a panic in a critical section is recovered (the inner value is
  returned, and the recovery is logged) instead of crashing the next caller.

## Store and schema lifecycle

`src/store/` is the only SQLite boundary. A fresh connection is opened per
tool call; WAL mode makes that cheap, and the connection's mode (read-only vs.
read-write) is fixed at open time and cannot change. Read tools take
`open_read_only`; the write tool takes `open_read_write`.

Three guards live at the open, because the open is the only place they can be
enforced atomically:

- **Symlinks.** Opened with `SQLITE_OPEN_NOFOLLOW` so a path that is or becomes
  a symlink is rejected by SQLite itself, with a pre-check for a clearer error
  in the common case.
- **Network filesystems.** `store/netfs.rs` refuses a database on NFS/CIFS/SMB
  (detected via `/proc/self/mountinfo` on Linux, the native inspection command
  on macOS; unknown platforms fail closed), because WAL locking is unreliable
  across those protocols.
- **Permissions.** A newly created database file is created owner-only in one
  syscall; an existing file readable or writable by group or other is
  rejected rather than silently exposed.

`store/connection.rs` also installs a SQLite authorizer that blocks
`ATTACH DATABASE` on every connection, so a read-only connection cannot reach
a second database file. This is the engine-level half of the invariant that
correctness comes from the connection, not from inspecting query text.

**Value-size ceiling.** The bundled SQLite caps one TEXT/BLOB at
`SQLITE_MAX_LENGTH` (1,000,000,000 bytes; `libsqlite3-sys` 0.35.0). The
per-file and per-query-value limits are clamped to that ceiling in the
Adaptive profile, env overrides, and project config, with a warning when an
override is reduced, so an over-limit file becomes a reported skipped row
rather than a failed publish. `max_total_import_bytes` is a sampled-bytes
**work budget**, not a memory bound; the separate memory/in-flight design is
a pending decision (PLAN, 2026-10-19).

**Schema versioning is discard-and-rebuild.** `store/migrations.rs` writes
`CURRENT_SCHEMA_VERSION` (currently `7`) to `PRAGMA user_version`. A database
whose version is neither fresh (`0`) nor current resolves as
`UnsupportedVersion`, which `is_corruption()` reports as corruption; the sync
layer discards the file and rebuilds it from project source. There is no
migration code path, and a version bump needs only a matching `schema.sql`
change — a bump is free because there is no primary data to preserve.

The database holds only current state: `files`, `evidence`, and
`dependency_edges` are replaced in place, never duplicated per revision, and
`revisions` is an append-only log of publish events used as freshness metadata
for the caller. Atomicity comes from SQLite's transaction semantics; a unique
partial index (`idx_revisions_single_current`) guarantees at most one current
revision, and readers bind to it and never observe a half-published one.

## Security and trust model

SlugAudit implements no authentication or authorization, and that is correct
for its deployment model. Here is why every layer of the trust boundary is
already handled by the environment rather than by code the server
reimplements.

### Process-level isolation is the transport boundary

The server speaks MCP over **stdio only** (`rmcp::transport::stdio`); stdout
is the JSON-RPC transport and logs go to stderr. It is spawned as a child
process by the agent, so only that parent can send it requests: there is no
network listener, no HTTP/SSE transport, no socket, and no multi-client
multiplexing. An attacker who wants to send a malicious MCP request must
already have code execution on the same machine and a handle to the same
process — at which point they can read any file the user can, including
`project.db` directly.

### The activation marker is an opt-in signal, not an authorization gate

Every tool call resolves a project by walking up from the given path looking
for `.planning/slugaudit/`; if the marker is absent the call fails with
`NotActive`. The marker's purpose is **discoverability and scope** — "which
project am I operating on?" — not authorization. The agent already has
filesystem access as the user that launched it, so SlugAudit grants no access
the agent did not already have; it makes queries fast.

A future network-accessible transport (SSE, WebSocket) would need its own
authentication layer. The stdio transport needs none because the process
boundary is the auth boundary.

### Credential awareness and log redaction

SlugAudit is designed to assist with auditing codebases. `report` surfaces a
`credential_pattern_file_count` so the agent is aware it is handling
credential-shaped content rather than silently reading secrets
(`src/tools/report.rs`), and `leads` can return the matching file paths. All
query parameters, finding text, and source contents are kept out of log
emissions (`src/tools/redaction_tests.rs`); the tool-call span logs only the
tool name and timing/outcome.

## Process lifecycle

SlugAudit is a **session-scoped child process**. It lives exactly as long as
the agent session that spawned it. When the agent disconnects (session ends,
model switch, the user closes the agent), it closes the server's stdin; the
server receives EOF on `waiting().await` and the process exits.

There is deliberately no graceful-shutdown handler. A SIGTERM → drain
handler is needed for a persistent daemon that serves multiple clients and
must not drop work. SlugAudit has exactly one caller. When that caller
disconnects, in-flight calls are abandoned but there is nobody to return
results to; a mid-publish kill leaves the database in a state WAL recovers or
the corruption path discards on the next open; the OS cleans up the process.
No work is dropped that had a recipient, and no primary data is lost.

## Performance characteristics

Benchmarks live in `benches/` (discovery, parsing, search, memory, startup)
with shared fixture generation in `benches/common/mod.rs`. Criterion is a
dev-dependency; benchmark builds are separate targets from correctness gates.
`check_performance` reruns the benches with a reduced sample size, compares
median point estimates against the committed baseline
(`.planning/archive/perf_baseline.json`), and fails when any bench regresses
past the threshold (default 20%); `--record` deliberately re-baselines.
Baselines are machine-class specific.

Two design consequences worth recording:

- **Search is a bounded scan, not an index.** Substring search runs as a
  `LIKE` over `files.content` / `evidence.payload`; there is no FTS5 table
  (see `benches/search.rs`). At the fixture sizes
  measured this is well inside budget, and FTS5 is the option to revisit only
  if scan cost grows super-linearly on much larger trees.
- **Unchanged sync writes nothing.** A manifest-identical publish is skipped,
  so the database grows only when content or the parser pack version actually
  changes. The database therefore tracks the current tree, not a history of
  it.

Operator-facing failure modes and their remedies — slow calls, full-publish
loops, sync-budget exhaustion, watcher limits, rebuild semantics — are in
`docs/TROUBLESHOOTING.md`.

## Testing layout

Tests live inline beside the code they cover, pulled in with
`#[cfg(test)] #[path = "..."] mod tests;`:

```
src/
└── module.rs                production code, ending with
                             #[cfg(test)] #[path = "module_tests.rs"] mod tests;
└── module_tests.rs          the sibling suite
```

The 72 `*_tests.rs` files share this pattern; `cargo test --lib` runs all of
them in parallel. Large suites are split into focused siblings (e.g.
`tools/finding_tests.rs` alongside `tools/finding_session_tests.rs`), and some
suites live in a `tests.rs` sibling instead (e.g. `watch/tests.rs`) — only
names ending in `_tests.rs` are counted by the gate. Per-file test-code
ceilings are enforced by `check_source_limits`; the ceilings themselves and
the exception-comment format are documented in `AGENTS.md`.

Log capture (`store::test_capture`) is thread-local but is **not** safe on
thread-locality alone. tracing's filtering state — each callsite's cached `Interest`
and the single `LevelFilter::current()` — is process-global, and tracing-core
recomputes both from one arbitrary thread's dispatcher whenever the registry holds
exactly one entry (`Rebuilder::JustOne`), where `NoSubscriber` answers
`Interest::never()`. A capture installing itself into that state silently loses its
own lines. `PERMISSIVE_BACKSTOP` is a permanently registered second dispatcher that
keeps the registry from ever holding one entry; it can only enable callsites, never
disable them, and must not be deleted as redundant. A test that **counts** captured
lines names its tool with a literal unique to it and filters the capture to that
literal; a test whose capture assertion is absence-only first asserts the capture is
live.

A large-repo soak test lives in `tests/large_repo_soak.rs` (an integration crate,
so `check_source_limits` does not count it). It builds 10,000 parseable modules and
syncs them through `SourceSyncManager::ensure_current` — the real publish path, no
shortcut — then asserts the sync completed inside `max_sync_wall_clock`, that all
10,000 files are `indexed` and parsed, and that the stored path set equals the
fixture's exactly. Measured on the 12-core Ryzen 9 5900X: 18.8 s against the
adaptive profile's 1200 s budget, so the assertion records headroom rather than
being tight — and it compares against the budget the sync actually enforced, so a
`lean` profile run checks against 120 s instead of passing regardless. The 10k case
is `#[ignore]`d (`cargo test --test large_repo_soak -- --ignored --nocapture`), and
a 64-file companion runs in the default suite in 0.13 s, so the same assertions are
guarded on every push and the shared runner's lines stay covered.

Fuzzing lives in `fuzz/`, a separate cargo workspace (its own manifest, lockfile and
`[workspace]` table), so nothing in it builds during `cargo build` / `cargo clippy` /
`cargo test` at the root — it needs nightly and a sanitizer. Three targets cover the
untrusted-input surfaces: `structure_query` (Tree-sitter query strings, which
`structure` executes as supplied), `query_sql` (agent SQL, including authorizer-bypass
shapes) and `request_arguments` (the `params.arguments` decode into the eight request
types — this repository has no frame parser to fuzz, because `src/main.rs` hands stdio
to `rmcp`, which owns framing). Bounded runs are clean at 219 / 316 / ~203k executions
per second, with peak RSS inside the 2048 MB limit. `fuzz/corpus/` is committed so a
scheduled run starts warm; `fuzz/target`, `fuzz/artifacts` and `fuzz/coverage` are
gitignored; `.github/workflows/fuzz.yml` runs them on a cron and on dispatch, never per
pull request. `fuzz/lsan-suppressions.txt` suppresses exactly one **dependency** leak
(tree-sitter 0.26.13's query compiler, minimal reproducer
`(identifier zzz: (identifier) @a)`); it is silently ineffective without
`llvm-symbolizer` and should be deleted when that pin stops leaking — the reasoning,
including why the frames prove the allocation is not this crate's, is in
`DEVIATIONS.md`.

## Design decisions and rejected alternatives

Each of these was considered and deliberately not done; the reason is part of
the architecture so the next reviewer does not re-litigate it from scratch.

- **The sync manager's split modules are its private children, not siblings.**
  `manager.rs` owns `SourceSyncManager`, `SyncedProject`, `SyncError`,
  constructors, counters, watcher registration and public observability;
  `manager_orchestration.rs` owns the outer `ensure_current` workflow — project
  resolution, database recovery, the shared rebuild tail — and `manager_health.rs`
  owns the `WatcherHealth` state machine and incremental barrier reconciliation,
  because those paths share health transitions, counters, progress signalling and
  freshness invariants. Both are `mod` declarations inside `manager.rs` rather than
  sibling public modules: that preserves private-field access, leaves the
  `sync::SourceSyncManager` API unchanged, and avoids widening encapsulation with
  `pub(crate)` state fields. A structural split only — the synchronization
  behaviour and its tests are unchanged.

- **No connection pooling.** A fresh connection per call is the correctness
  boundary: its read-only/read-write mode is set at open time and cannot
  change. Pooling would require tracking each connection's mode and handling
  per-connection corruption, for no measured benefit under WAL.
- **No watcher heartbeat thread.** The `notify` error callback already covers
  the failures `notify` reports (queue overflow, watch removal), and a
  silently dead watcher is self-correcting: the stat sweep and the
  `NeedsVerification` full publish both re-verify from disk, and the watcher
  is re-created on every agent session.
- **No graceful shutdown** (see *Process lifecycle*).
- **No HTTP health endpoint or metrics endpoint.** stdio is the only
  transport; the `health` MCP tool returns watcher health, unreconciled
  counts, tool-call counters, and sync timing over that same transport. An
  HTTP surface belongs with a network transport, not with stdio.
- **Findings are session-scoped.** A finding is a conclusion from one
  reasoning session, not a durable fact. A different agent must not silently
  inherit another session's conclusions; evidence persists because it is
  reproducible from source, findings do not (`sync/manager_meta.rs::
  purge_prior_session_findings`, invariant 9).
- **Findings are bound to a content hash, not a line range alone.** The hash
  is what makes auto-invalidation reliable when a file changes; the line range
  is a hint for the reader.
- **The `query` ATTACH guard is discipline, not containment.** The agent could
  read any file it has permission for with `sqlite3` directly. The authorizer
  exists so the "correctness comes from the connection" invariant stays true
  if the tool surface changes (see *Store and schema lifecycle*).
- **Progress delivery is fire-and-forget.** Each notification spawns a tiny
  future and sends are best-effort, so a broken progress channel cannot turn a
  successful call into an error. Volume is throttled by `McpProgressSink`.
- **The database is discarded, not migrated.** See *Store and schema
  lifecycle*.

## Key invariants

These constraints shape every module above; removing or weakening one breaks
something else loudly.

1. **Correctness comes from the connection itself, never from inspecting
   query text.** Read paths open `SQLITE_OPEN_READ_ONLY`; the `query` tool
   never reaches a read-write connection, and the ATTACH authorizer blocks the
   one read a read-only connection would otherwise permit.
2. **Symlinks and network filesystems are rejected at open time.** A
   symlinked database returns `StoreError::Symlink`; one on NFS/CIFS/SMB
   returns `StoreError::NetworkFilesystem`. The open is the only check.
3. **All `Mutex` locks go through `lock_or_recover`.** A poisoned mutex is
   recovered and logged, not re-panicked, so a bug in watcher state mutation
   cannot take down the server.
4. **The barrier sync loops at most `MAX_BARRIER_LOOPS = 16` times.** A racing
   producer is detected, the watcher is marked `Desynced`, and
   `ReconcileError::BarrierCapExceeded` is returned rather than looping
   forever.
5. **Tool work runs on the blocking pool, gated by a per-server
   `Arc<Semaphore>` of 8 permits.** Slow I/O never starves the async runtime;
   concurrent calls fan out across the permit pool.
6. **Each revision is published atomically under compare-and-swap.**
   `revision::publish_revision` upserts files and evidence, deletes removed
   files, and flips `is_current` in one transaction; a concurrent publisher
   detects the mismatch and retries (bounded by `MAX_CAS_RETRIES = 4`).
7. **Findings auto-invalidate on file change.** A finding is stored with the
   file's current `content_hash`; when that hash changes, the finding becomes
   `stale` inside the publish transaction and is not served.
8. **`#![forbid(unsafe_code)]` is enforced crate-wide**, alongside
   `deny(unreachable_pub, trivial_casts, trivial_numeric_casts,
   elided_lifetimes_in_paths)` and `deny(clippy::print_stdout)` — stdout is
   the JSON-RPC transport, so a stray `println!` in serve-reachable code is a
   compile error, not a corrupted stream.
9. **Findings are scoped to the agent session that wrote them.** Every finding
   row carries the active `session_id` (a UUID generated once per process);
   `purge_prior_session_findings` runs as part of the first sync,
   deleting rows from any other session. A new agent starts with a clean
   finding set rather than silently inheriting another's conclusions.
10. **The database is disposable derived data.** Every row in `files`,
    `evidence`, `dependency_edges`, and `revisions` is computed from project
    source and can be regenerated: `rm -rf .planning/slugaudit` followed by any
    tool call. Findings are session-scoped (invariant 9) and hash-bound
    (invariant 7), so they follow the same philosophy: conclusions are
    ephemeral, evidence is reproducible.
11. **Production code never panics on an error value.** `unwrap` and `expect`
    are denied outside `cfg(test)` at the crate root and in every gate binary,
    so fallible work returns `Result` and a genuinely fatal condition reports
    the error and chooses an exit code. Together with invariant 3 this is why
    a defect surfaces as one failed tool call — `server_runner` maps a
    panicking blocking task to `ErrorData::internal_error("tool task
    failed: …")` — rather than a dead server. A panic in a test is exempt: it
    *is* the failure signal.
12. **Every count a tool returns is quotable as-is.** If a number needs a
    caveat the response does not carry, it is wrong. The concrete form: a
    `files` row is written for every discovered file, including the ones that
    were skipped (`file_kind = 'skipped'`, `skip_reason`, no content), so
    `count(*)` cannot silently omit them; and `report.completeness` states the
    breakdowns (kind, class, skip reason, extraction completeness) and a
    partial-index flag beside the counts they qualify. No response carries a
    score, a severity, or a ranking — evidence is not judgment.
13. **A failing tool call carries a machine-readable code.** A tool error
    may attach `{"code": …, "remedy": …}` in `ErrorData.data` — the only
    machine-readable channel rmcp gives a response. The code is a member of
    the closed set below, and it is attached where the *typed* failure still
    exists (discovery, publish, reconcile, sweep, project activation), never
    guessed from a message at the outermost tool. Because a code a caller
    cannot act on is noise, the set is small and every remedy is an action:
    the sync-budget remedy names the knob rather than advising a retry that
    fails identically, and a path that is not an enabled project produces one
    `project_not_enabled` instruction instead of one opaque failure per tool.

### Machine-readable failure codes

The closed set lives in `src/error_codes.rs`; this table is checked against
the test-local `#[cfg(test)] pub(crate) const ALL` in both directions by a
unit test there, so a code cannot be added without its row and a row cannot
outlive its code. Codes are lowercase
`snake_case`, name the condition rather than the call site, and are stable
once shipped — renaming one breaks every caller that branches on it. Remedies
are static strings: **never** interpolate a path, SQL, query string, finding
body, or file content into a code or a remedy (the redaction contract in
`tools/redaction_tests.rs` keeps that text off every logged or mirrored
surface).

<!-- error-codes:begin -->
<!-- Machine-read: `src/error_codes.rs::the_documented_table_matches_the_code_set_exactly`
     parses this block and asserts the rows below are exactly the test-local
     `#[cfg(test)] pub(crate) const ALL`, in declaration order. Add, remove or
     reorder a code here in the same change that
     changes the enum, or `cargo test` fails. The module map above and the
     `*_tests.rs` count sentence are read the same way, by `check_docs_drift`. -->
| code | meaning | attached where |
| --- | --- | --- |
| `sync_budget_exhausted` | the sync wall-clock budget (`max_sync_wall_clock`) was spent while discovering, hashing, parsing, or publishing; permanent on a tree too big for the budget, so the remedy names the knob | every sync map site: `manager_meta::map_publish_error`, `sweep_reconcile::sweep_tree` and `sweep_reconcile::apply_sweep_report`, both `manager_health` map sites |
| `project_not_enabled` | no ancestor of the requested path has an activation marker, so no tool can serve it | `manager_orchestration::map_activation_error` — the single funnel behind every tool |
| `stale_revision` | the revision this call was pinned to is no longer current, or a sampled file changed before the write — the compare-and-swap race, where retrying is right | `tools::context::verify_revision_matches`, `manager_meta::map_publish_error`; also `tools::context::guard_expected_revision`, where the caller pinned a resumed page (`leads`/`query` `expected_revision`) and the remedy restarts the sweep from offset 0 instead of retrying the offset |
| `unknown_mechanic` | `leads` was asked for a mechanic id that is not in the catalogue | `tools::leads_mechanics::resolve_mechanic` |
| `sql_too_large` | the SQL passed to `query` exceeded `max_query_sql_bytes` | `tools::query::query_with_limits` |
| `file_not_indexed` | the named file has no indexed source content (never discovered, skipped as oversize/unreadable, or binary) | `tools::structure_source::fetch_source`, `tools::finding::fetch_file` |
<!-- error-codes:end -->

`health` is the deliberate exception: it answers normally in the not-enabled
state (`phase: "disabled"` plus the same `remedy`) instead of failing, because
it is the tool a caller reaches for precisely when nothing works.

## Where to start reading

1. `README.md` — install, connect, license.
2. `AGENTS.md` — why this exists, the founding metric, verified conventions,
   and the gate suite.
3. **This file** — structure, invariants, and the reasoning behind them.
4. `src/main.rs` + `src/server.rs` + `src/server_runner.rs` — what actually
   happens when a tool call arrives: tool contracts in `server.rs`,
   semaphore-bounded dispatch and progress in `server_runner.rs`.
5. `src/sync/manager.rs` with its `sync/manager_orchestration.rs`,
   `sync/manager_health.rs`, and `sync/manager_meta.rs` children — the
   synchronization state machine.
6. `src/sync/sweep.rs` + `src/sync/reconcile/` — the freshness guarantee.
7. `src/graph/resolver/` — how imports become dependency edges.

If something in the codebase disagrees with this document, fix one or the
other — never both.
