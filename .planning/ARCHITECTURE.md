# SlugAudit Architecture

> **Product boundary:** SlugAudit is shipped to users as the single
> `slugaudit-mcp` binary. It runs inside a user's project through an
> MCP-compatible agent and stores disposable derived data in
> `<project-root>/.planning/slugaudit/project.db`. In this repository,
> `.planning/` contains SlugAudit's development documentation; in a customer
> project, `.planning/` is customer-owned project data. SlugAudit owns only the
> `slugaudit/` child directory and excludes it from discovery.
>
This document is the single high-level source of truth for how SlugAudit
fits together. It is the first thing a senior engineer should read when
joining — `.planning/PHASE-*.md` files document specific work,
`OBSERVABILITY.md` documents telemetry behavior, `PACKAGING.md` documents
release mechanics, and `CLAUDE.md` (if present at any point) documents
agent-specific notes. **This** file documents what the system *is*.

## High-level overview

SlugAudit is a stdio-MCP server (`slugaudit-mcp serve`) backed by a
per-project SQLite database. Every enabled project gets its own
`project.db` in `./<project>/.planning/slugaudit/`. The server is
stateless across projects: enabling a project on a sibling is a
brand-new SQLite handle; cross-project questions go through the orchestrating
agent, not through SlugAudit.

The server exposes seven tools:

- **`report`** — read-only snapshot of the indexed revision:
  file/language counts, parser failures, evidence-kind counts.
- **`query`** — read-only arbitrary SQL on the project's own database.
  The connection itself refuses writes; the SQL is never parsed for
  write intent.
- **`structure`** — Tree-sitter structural pattern matching against
  indexed source content (300+ languages).
- **`finding`** — the single write tool. Persists AI-authored findings
  bound to a file's current `content_hash`, auto-invalidating on file
  change.
- **`finding_read`** — returns findings scoped to the current agent
  session. Unlike raw `query` against the `findings` table (which
  returns every session's rows), `finding_read` gates on the current
  `session_id` — a new agent sees only its own conclusions.
- **`project_control`** — enable/disable a project (creates or removes
  the activation directory and runs the initial import).
- **`health`** — operational snapshot: watcher health, unreconciled
  counts, cumulative tool-call counters, last-sync timestamp.

Of these, only `finding` and `project_control` mutate state; the rest
are read-only views into the most recent revision's evidence. All
state-bearing tools synchronize the relevant project before serving —
if the watcher reports untrusted state (restart, integrity violation)
the next tool call does a full publish instead of trusting the
dirty-set.

## Repository versus runtime scope

The `.planning/` directory at this repository root contains development
material for SlugAudit. It is not shipped as part of the end-user binary.
When the binary runs in a customer project, however, that project's
`.planning/` directory is customer data and may be indexed. Only
`<project-root>/.planning/slugaudit/` is application-owned runtime state; its
`project.db` is disposable derived data.

See the repository-level [DEVELOPMENT_SCOPE.md](../DEVELOPMENT_SCOPE.md) for
the complete scope policy.

## Module map

```
src/
├── main.rs                   CLI dispatch (parse_args → Connect/Install/Serve)
├── lib.rs / mod.rs           crate root, top-level pub mod declarations
├── server.rs                 rmcp ServerHandler + tool router registration
│                             (tool contracts only)
├── server_runner.rs          run_blocking worker pool (semaphore-bounded) +
│                             MCP progress sink plumbing
├── cli.rs                    Command enum + parse_args + USAGE string
├── connect.rs                register the running binary with a target agent
├── install.rs                copy the binary to a stable ~/.slugthug/bin path
├── menu.rs                   interactive setup CLI (install/connect/other-client/run server)
├── util.rs                   cross-module helpers (lock_or_recover, hex_encode, …)

├── tools/                    MCP tool handlers — thin orchestration:
│   ├── mod.rs                ToolCounters, public exports
│   ├── context.rs            ensure_synced() + transaction envelopes
│   ├── context_transactions.rs with / with_verified_write helpers
│   ├── report.rs             automatic snapshot
│   ├── query.rs              arbitrary read-only SQL with budgets
│   ├── query_value.rs        SQLite row → JSON conversion + size cap
│   ├── structure.rs          tree-sitter pattern match
│   ├── finding.rs            the single write tool
│   ├── finding_read.rs       session-gated finding query
│   ├── project_control.rs    enable/disable
│   └── health.rs             operational snapshot (Phase 2.1)

├── sync/                     Indexing + watcher-aware reconciliation
│   ├── mod.rs                public exports
│   ├── manager.rs            SourceSyncManager: ensure_current, reconcile, health accessors
│   ├── manager_meta.rs       current_revision_id + ensure_project_row helpers
│   ├── reconcile/             dirty/deleted reconciliation — split into 6 files (mod.rs + error/report/options/pipeline/barrier/queries)
│   │   ├── mod.rs             re-exports + MAX_BARRIER_LOOPS
│   │   ├── error.rs           ReconcileError
│   │   ├── report.rs          ReconcileReport
│   │   ├── options.rs         ReconcileOptions (budget + ignore rules)
│   │   ├── pipeline.rs        per-path dirty-file loop
│   │   ├── barrier.rs         event-barrier sync loop
│   │   └── queries.rs         SQL helpers (existing hashes, parser pack version)
│   ├── discovery.rs          filesystem walk with extension/limit filters
│   ├── hash.rs               BLAKE3 content hashing
│   ├── sample.rs             read-file-with-budgets
│   ├── sample_batch.rs       parallel batch sampling loop + per-file skips
│   ├── publish.rs            drive a single publish attempt
│   ├── publish_attempt.rs    first-try wrapper
│   ├── publish_cas.rs        compare-and-swap retry primitive
│   ├── publish_diff.rs       before/after diff for diagnostics
│   ├── publish_log.rs        retry counter + warn! on each retry
│   ├── revalidate.rs         re-sample previously-upserted files
│   ├── manifest.rs           manifest hash computation
│   ├── revision.rs           revisions table writes (atomic current-pointer swap)
│   ├── revision_edges.rs     dependency_edges table writes
│   ├── analyze.rs            tree-sitter parse for one file
│   └── race_hook.rs          test-only injection hook

├── graph/                    import → project-file resolution
│   ├── mod.rs                public exports + dispatch entry point
│   ├── reference.rs          ImportReference (pre-resolution extraction)
│   ├── resolve.rs            path-arithmetic primitives
│   ├── resolve_rust.rs       Rust-specific `crate::`/`super::`/`self::` resolution
│   └── resolver/             generic + per-language resolvers — split into 5 files (mod.rs + generic/python/js/path_helpers/registry)
│       ├── mod.rs            re-exports + module map (entry point)
│       ├── generic.rs        Resolution/ResolutionKind, LanguageResolver trait, GenericResolver
│       ├── python.rs         Python-style relative imports + __init__.py
│       ├── js.rs             JS/TS-style `import … from 'path'`
│       ├── path_helpers.rs   extract_quoted_string, candidate_paths, …
│       └── registry.rs       OnceLock-backed registry + get_resolver/resolve_one

├── watch/                    filesystem watching
│   ├── mod.rs                module map
│   ├── state.rs              WatchState concurrency wrapper (locks go through lock_or_recover)
│   ├── types.rs              pure data types: WatcherHealth enum, ProjectWatchState struct
│   ├── path.rs               normalize_relative_path helper
│   ├── manager.rs            WatchManager: event dispatch + project registry
│   └── tests.rs              integration tests

├── project/                  project resolution + activation
│   ├── mod.rs                find_project_root, enable/disable
│   ├── root.rs               ProjectRoot type
│   ├── activation.rs         write/read the activation dir marker
│   └── database_path.rs      where this project's project.db lives

├── store/                    SQLite connection boundary
│   ├── mod.rs                open_read_write/open_read_only/discard_corrupt_database
│   ├── connection.rs         typed StoreError + symlink/NFS/perm guards
│   ├── netfs.rs              network-filesystem rejection (Linux macOS)
│   ├── migrations.rs         schema versioning
│   └── schema.sql            canonical schema

├── parse/                    tree-sitter parser registration
├── model/                    ResourceLimits + evidence/span types
├── evidence/                 evidence kind/category enums + normalize helpers
│   └── calls.rs              Rust call-site evidence walker
└── cli_tests.rs etc.         inline #[path] test modules per source file
```

## File-size authorization summary

Every file under `src/` is governed by `cargo run --bin check_source_limits --locked` (the bin that replaced the prior `tools/check_source_limits.sh` shell script). Production files at 0–199 code lines auto-pass the gate; 200–300 requires an in-source comment of the form

```rust
// slugaudit-line-exception: approved-by=<who>; reason=<why>
```

and ≥300 hard-fails the gate (CI red). Test files (`*_tests.rs`, `tests.rs`) enumerate one named behavior per `#[test]` — repetition that cannot be DRY'd without losing failure diagnostics — so they auto-pass up to 500 code lines and hard-fail above, with no exception step. The scopes below mirror those stages as OAuth-style decisions so a reviewer can see at a glance which files are explicit grants versus implicit auto-passes versus active violations — without re-running the script.

| Stage | Scope decision | Count |
|---|---|---|
| 0–199 LoC (any file) | `source-size:auto` | 139 |
| test files 200–500 LoC | `source-size:test-auto` | 9 |
| production 200–300 LoC, annotated | `source-size:approved-exception` | 10 |
| production 200–300 LoC, **NOT** annotated | `source-size:violation` | 0 |
| >300 production / >500 test LoC | `source-size:hard-fail` | 0 |

### Approved exceptions — `source-size:approved-exception`

The following **production** files exceed the 200 LoC soft cap with an
`approved-by=agent; reason=…` annotation justifying the bundling.
Test files between 200–500 lines need no annotation under the
test-file rule above.

| File | LoC | Reason |
|---|---:|---|
| `src/bin/check_no_duplicates.rs` | 279 | two orthogonal gate inputs (commit-subject dedupe via `git log`, `#[test]` name dedupe) share one argv parser, exit-code contract, and failure-printer |
| `src/bin/check_performance/main.rs` | 249 | criterion argv, per-row regression comparison with budget tracking, and the verdict printer share one process so the single user-visible CLI output stays coherent |
| `src/bin/check_source_limits/counter.rs` | 264 | the token-aware counter's comment/string/char/raw-string states are one atomic scanner; splitting the state machine would fragment the exact token-coverage semantics the gate depends on |
| `src/evidence/normalize.rs` | 224 | one match arm per Tree-sitter evidence kind; splitting by kind would hide the exhaustiveness this file exists to guarantee |
| `src/graph/resolve_rust.rs` | 210 | one resolution pipeline per Rust import form (workspace anchoring, super/self walk, item-vs-module shortening) with mutually recursive helpers on the same `known_paths` contract |
| `src/store/migrations.rs` | 243 | one migration per schema version plus version-pinning tests form a single forward-only sequence; splitting would scatter the ordering invariant (and the exact-version pin) |
| `src/sync/manager.rs` | 291 | `ensure_current`'s three-branch match is the sync orchestrator's hot path; trace sites + `stamp_last_sync` belong next to the code paths they cover |
| `src/tools/health.rs` | 236 | health is the schema-defining tool; Request + Response + phase + derivation live together so the schema isn't split from its only consumer |
| `src/tools/query.rs` | 261 | one tool contract owns request/response types, the execution/budget path, and the separator scanner; splitting would fragment the validation order (empty → size → statement count → freshness → budget) the tests assert against |
| `src/watch/manager.rs` | 287 | one file owns the notify watcher lifecycle, per-project watch states, scope/rule maintenance, and the event filter; splitting would fragment the manager's lock discipline and the unwatch rule |

## Data flow (one tool call)

```
Caller                             SlugAudit server
└─ MCP request                     run_blocking (semaphore-bounded)
   └─ tools::ensure_synced         → SourceSyncManager::ensure_current
      │                              <!-- mcp://progress: ensuring_current (Phase 2.2 wire point) -->
      ├─ project::find_project_root  search parents for `.planning/slugaudit`
      ├─ store::open_read_write      symlink/NFS/perm reject; WAL; migrations
      ├─ ensure_project_row          INSERT OR IGNORE the singleton metadata row
      ├─ watch_manager.watch         register or reuse the project's WatchState
      └─ match health branch         │
         ├─ NeedsVerification│Desynced│Unavailable → publish::publish (full)
         │                              <!-- mcp://progress: publishing {i}/{total} per file sampled -->
         │                          then reconcile drained events + set Healthy
         └─ Healthy + dirty events → reconcile (barrier-bounded to MAX_BARRIER_LOOPS)
            └─ Healthy + no events → read current revision_id from revisions
   └─ tools::* handler             read-only context-transaction or finding write
   └─ ToolCounters::record_call    bump call_count / total_ms / error_count
   └─ tracing::info!|warn!         log completion (or failure) with counters in span
   └─ Json<{…}>                    return
                                       <!-- mcp://progress: completed -->
Caller receives JSON response
```

> **MCP progress annotations** (the `<!-- mcp://progress: ... -->` Slack-style inline
> comments in the diagram): these mark the live wire points where MCP
> `/notifications/progress` events are emitted by
> `server_runner::run_blocking` — `0.0` with message
> `{tool} ensuring_current` before the semaphore acquire, `0.5` with
> `{tool} publishing` once the permit is held (per-file `Sampling`
> events from the sync layer's `McpProgressSink` then overwrite this
> with the real i/N ratio), and `1.0` with `{tool} completed` when the
> work finishes, whether it succeeded or failed. Notifications are
> best-effort: a broken progress channel can never fail a successful
> tool call. Labels that differ slightly from these messages (e.g. a
> reconcile stage between publishing and completed) reflect phases the
> sync layer reports through its own `ProgressEvent` stream rather than
> a separate wire point.

## Key invariants

These constraints shape every module above; remove or weaken one and
something else breaks loudly.

1. **Correctness come from the connection itself, never from inspecting
   query text.** Read tools open a `SQLITE_OPEN_READ_ONLY`, write tools
   open a `SQLITE_OPEN_READ_WRITE` but the `query` tool path never
   reaches for the write connection.
2. **Symlinks and network filesystems are rejected at open time.** A
   symlinked `project.db` returns `StoreError::Symlink`; an `project.db`
   on NFS/CIFS/SMB returns `StoreError::NetworkFilesystem`. The
   open is the only place this is checked — every other tool trusts the
   connection.
3. **All `Mutex` locks go through `lock_or_recover`.** A poisoned mutex
   after a panic inside a critical section is recovered (inner value
   returned) instead of crashing the next caller. This is the
   single-line reason a bug in watcher state mutation can't take down
   the entire MCP server.
4. **The barrier sync loops at most `MAX_BARRIER_LOOPS = 16` times.**
   A racing producer (editor auto-save, fsmonitor storm) is detected,
   `WatcherHealth::Desynced` is set, and `ReconcileError::BarrierCapExceeded`
   is returned rather than looping forever and exhausting memory.
5. **Tools use `run_blocking` on Tokio's blocking thread pool, gated by
   an `Arc<Semaphore>` of 8 permits.** Slow I/O never starves the
   async runtime; concurrent tool calls fan out across the permit pool.
6. **Each revision is published atomically.** `revision::publish_revision`
   computes the manifest hash, opens a write transaction, upserts files,
   inserts the revision row, and updates `revisions.is_current` in one
   txn. Concurrent publishers detect the mismatch via compare-and-swap
   and retry.
7. **Findings auto-invalidate on file change.** A finding is stored with
   the file's current `content_hash`; the moment that hash changes, the
   finding becomes `stale` and won't be served. The auto-invalidation
   runs on the next `ensure_synced`/publish pass.
8. **`#![forbid(unsafe_code)]` is enforced crate-wide.** The CI
   `cargo deny` and `clippy -D warnings` checks enforce this, so a
   reviewer doesn't have to verify it on every PR.
9. **Findings are scoped to the agent session that wrote them.** Every
   finding row carries the active `session_id` (a UUID generated once
   per `slugaudit-mcp` boot). Every `ensure_current` runs
   `purge_prior_session_findings` (`sync::manager_meta`) as the first
   step inside `ensure_project_row`, deleting rows whose `session_id`
   does not match the current process. A new agent (new chat, new
   model, new reasoning context) starts with a clean finding set —
   it does not silently inherit another session's audit conclusions.
   The defense fires on both the `ensure_synced` read path and the
   `publish_from_scratch` recover-from-corruption path.
10. **The database is disposable derived data.** Every row in `files`,
    `evidence`, `dependency_edges`, and `revisions` is computed from
    the project's source files. The database can be deleted and
    rebuilt with no data loss: `rm -rf .planning/slugaudit` followed
    by any tool call. This is not a compromise — it is the correct
    architecture for a cache whose input is always available and
    whose output is always reproducible. The `findings` table is
    session-scoped (invariant #9) and auto-invalidates on source
    change, so it follows the same philosophy: conclusions are
    ephemeral, evidence is reproducible.

## Security and trust model

SlugAudit does not implement authentication or authorization — and
that is correct for its deployment model. Here is why every layer
of the trust boundary is already handled by the environment, not by
code the server reimplements.

### Process-level isolation (the transport boundary)

The server speaks MCP over **stdio only** (`rmcp::transport::stdio`).
It is spawned as a child process by the AI agent. Only the parent
process that launched it can send it MCP requests. There is no
network listener, no HTTP/SSE transport, no socket, and no
multi-client multiplexing. An attacker who wants to send a malicious
MCP request must already have code execution on the same machine and
a handle to the same process — at which point they can `cat` any
file the user can read, including the project database directly via
`sqlite3`.

### The activation marker is an opt-in signal, not an authorization gate

Every tool call resolves a project root by walking up from the
given `path` looking for `.planning/slugaudit/`. If the marker
doesn't exist, the call fails with `NotActive`. But this marker's
purpose is **discoverability and scope** — "which project am I
operating on?" — not authorization. The AI agent already has
filesystem access as the user that launched it. If the agent wanted
to read files from an unindexed project, it could `cat` them
directly. SlugAudit makes queries fast; it does not grant access the
agent didn't already have.

A future network-accessible transport (SSE, WebSocket) would need
its own authentication layer. The current stdio transport needs
none because the process boundary is the auth boundary.

### The database is disposable derived data

Every byte in `project.db` — files, content hashes, evidence,
dependency edges — is computed from the project's source files. The
database is a cache. If it is corrupt, it is discarded and rebuilt
(`store::discard_corrupt_database`). If it is deleted, it is
re-created on the next tool call. The only non-derived data is
the `findings` table (AI-authored conclusions), and those are
session-scoped (purged on fresh boot) and auto-invalidated on
source change.

This means entire categories of production concerns do not apply:

- **Rollback:** `rm -rf .planning/slugaudit`. There is no primary
  data to preserve.
- **Backups:** the source tree is the backup. Re-index it.
- **Connection-close errors:** a failed WAL checkpoint on drop is a
  non-issue — the next open recovers or discards.
- **Schema downgrades:** unsupported — delete the database and
  let the new binary rebuild it.

## Process lifecycle

SlugAudit is a **session-scoped child process**. It lives exactly as
long as the AI agent session that spawned it. When the agent
disconnects (session ends, model switch, user closes the agent),
it closes the server's stdin. The server receives EOF on
`waiting().await` and the process exits.

### Why there is no graceful shutdown

A graceful-shutdown handler (SIGTERM → stop accepting new calls →
drain in-flight → exit) is needed for a persistent daemon that
serves multiple clients and must not drop work. SlugAudit is not a
daemon. It is a child process with exactly one caller. When the
caller disconnects:

- stdin closes → the server exits.
- In-flight tool calls are abandoned, but the caller is gone —
  there is nobody to return results to.
- The database is disposable — a mid-publish kill leaves the
  database in a state that SQLite's WAL recovers or the
  corruption path discards on the next open.
- The OS cleans up the process.

No work is dropped that had a recipient. No primary data is lost.
A signal handler that drains in-flight calls would add complexity
for a scenario that cannot occur in this deployment model.

## Disposable-data philosophy

SlugAudit's database is **derived, not primary**. The design
deliberately trades durability for simplicity in every data path:

| Concern | Traditional approach | SlugAudit approach |
|---|---|---|
| Database corruption | Backup + restore | Discard + republish from source |
| Schema change | Migration + rollback plan | Forward-only migration; delete DB to go back |
| Connection errors | Retry + alert | Reopen; if corrupt, discard + rebuild |
| Data loss | Replication + snapshots | Source files are the canonical copy |
| Upgrade/downgrade | Compatibility matrix | Newer schema → reject. Older schema → migrate. No downgrade path. |

This is not a compromise — it is the correct architecture for a
tool whose input (source files) is always available and whose
output (cached evidence) is always reproducible. The integrity
check is simple: at any time, delete the database and run one tool
call. If the evidence comes back identical, the system is correct.

The `findings` table is the exception: AI-authored conclusions
persist within a session. Those are scoped to the process that
wrote them and purged on the next boot (invariant #9), so even
findings do not require durability across process lifetimes.

## Concurrency and deadline model

### The semaphore gates blocking-pool permits, not work duration

Tool calls are dispatched through `server_runner::run_blocking`,
which acquires a permit from an `Arc<Semaphore>` of 8 permits
before moving work onto Tokio's blocking thread pool (invariant #5).
The acquire is an **async yield** — it parks the calling task on a
Tokio worker thread and yields to other tasks. It does not block a
thread. A caller waiting for a permit is waiting in the async
runtime, not consuming a blocking-pool thread.

### Why the semaphore does not need its own timeout

A naive audit flags `semaphore.acquire_owned().await` with no
`.timeout()` as a hang risk. In practice:

- **Every blocking work closure has its own deadline.** Sync
  operations check `Deadline::exceeded()` at each cooperative
  point (per discovered file, per dirty path, per barrier
  iteration). `query` has a 5-second wall-clock budget enforced
  through SQLite's progress handler (which fires during statement
  execution, not between statements). `structure` has a 5-second
  budget enforced through Tree-sitter's native progress callback.
- **The per-operation deadlines bound permit hold time.** A permit
  cannot be held indefinitely because the work it guards will
  finish or fail under its own budget.
- **The single-client model means the pool is rarely contended.**
  An MCP agent makes one tool call at a time (request-response).
  Multiple concurrent permits are headroom, not a load-bearing
  concurrency mechanism.
- **The acquire is async, not blocking.** If no permit is
  available, the calling task yields. The async runtime schedules
  other work. The semaphore is a fairness mechanism, not a
  bottleneck.

Adding a timeout to the acquire would be defense-in-depth against
a scenario (all 8 blocking workers stuck in uninterruptible kernel
syscalls simultaneously) that the kernel's own I/O timeouts already
handle. It is harmless to add, but its absence is not a defect.

### Runtime resource-limit configuration

Every field in `ResourceLimits` can be overridden at startup via
an environment variable. The pattern is `SLUGAUDIT_<FIELD>` where
`<FIELD>` is the SCREAMING_SNAKE_CASE name of the field:

- `SLUGAUDIT_MAX_FILE_BYTES` — per-file size cap (default 8 MiB)
- `SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES` — total import byte cap (256 MiB)
- `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES` — query response JSON size cap
- `SLUGAUDIT_MAX_QUERY_SQL_BYTES` — max SQL text length
- `SLUGAUDIT_MAX_QUERY_VM_STEPS` — SQLite VM-step budget
- `SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS` — query wall-clock budget (seconds)
- `SLUGAUDIT_MAX_QUERY_VALUE_BYTES` — per-column value cap
- `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES` — tree-sitter query text length
- `SLUGAUDIT_MAX_STRUCTURE_MATCHES` — max structure matches returned
- `SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS` — structure time budget
- `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` — sync time budget (seconds)

Unset or unparseable vars are silently ignored — the compile-time
default applies. Duration fields accept whole seconds. The limits are
cached in a `OnceLock` on first use via `model::process_limits()` and
never change for the lifetime of the process.

## Watcher health model

The filesystem watcher (`src/watch/manager.rs`) runs on `notify`'s
internal event loop. Its callback is deliberately non-blocking:
`try_lock()` to acquire the manager's state, then quick `HashSet`
inserts. If the lock is held by the sync layer, the event is
silently dropped — the sync layer will re-verify on the next
`ensure_current`, so a dropped event is harmless.

### What the error callback catches

The watcher's `Err` arm (line ~116 of `manager.rs`) transitions
every project to `WatcherHealth::Desynced` when `notify` reports
a queue overflow, watch removal, or other integrity problem. The
next `ensure_current` sees `Desynced` and does a full publish.
This catches every watcher failure `notify` can report.

### Why there is no separate health heartbeat

A true watcher heartbeat would need a **separate watchdog thread**
— a thread whose only job is to check that the notify event loop
is still delivering events. The notify callback cannot heartbeat
itself (if the loop is dead, the heartbeat never fires). A
watchdog thread adds complexity for a failure mode that:

- `notify`'s own error callback already covers (inotify queue
  overflow, watch removal).
- Is self-correcting: if the watcher silently stops, the user or
  AI agent notices stale evidence and restarts the server, which
  triggers `NeedsVerification` → full publish.
- Is a `notify` crate bug, not a SlugAudit bug — and the
  session-scoped process lifetime means the watcher is re-created
  on every agent session anyway.

The `health` MCP tool exposes `watcher_health` and
`consecutive_full_publishes` — an operator monitoring those fields
can see that the watcher is being trusted (incremental reconcile)
vs. falling back to full publishes (distrust). This is the
observability surface that matters.

## Design FAQ — "Why didn't you do X?"

Every question below is something a commercial audit or architecture
review will ask. If a new question comes up during review, add it here
with the answer so the next auditor finds it immediately.

### Q: What does Rust call evidence guarantee?

**Answer:** Rust call evidence is syntax evidence produced from the Rust
Tree-sitter grammar. It records call spans, callee text, a simple callee name,
and the enclosing function when available. It does not claim exact runtime
resolution for trait dispatch, generics, function pointers, closures, macros,
or generated code. Agents should use the `Call` evidence rows to locate
likely call sites, then inspect the stored source before drawing conclusions.

The implementation lives in `src/evidence/calls.rs`. Additional language
support must preserve explicit uncertainty rather than inventing targets.

### Q: Why is there no separate watcher heartbeat?

**Answer:** `ensure_synced` runs before every state-bearing tool call
(see Data Flow diagram). If the watcher is healthy and has pending
events, it incrementally reconciles. If the watcher is Desynced or
Unavailable, it does a full publish from disk. A "silently dead"
watcher means events aren't delivered → next call does a full publish
(correct, just slower). The staleness window is bounded by the
session-scoped process lifetime — the watcher is re-created on every
agent session anyway. A watchdog thread would add complexity for a
failure mode (`notify` crate bug — thread exits without firing error
callback) that is both extremely rare and self-correcting.

Full rationale: "Watcher health model" section above.

### Q: Why `try_lock()` in the watcher callback? Doesn't that drop events?

**Answer:** Yes, and that's correct. The watcher callback runs on
`notify`'s internal event loop. Blocking that loop (via `lock()`)
would stall event delivery for every project. A dropped event is
harmless — the next `ensure_synced` will see unreconciled events or
do a full publish. Dropping is the recovery path, not a bug.

Full rationale: "Watcher health model" section above.

### Q: Why is logging human-readable text instead of structured JSON?

**Answer:** SlugAudit speaks MCP over stdio. Stdout is the JSON-RPC
transport. Stderr is piped to the MCP host's own log viewer — a
human. ANSI is disabled because the host's log viewer isn't a
terminal. When/if SSE/HTTP transport is added, flipping to JSON
output is a one-liner (`tracing_subscriber::fmt().json()` behind an
env var). Until then, human-readable stderr is the correct format
for the single-human-consumer deployment model.

Full rationale: `OBSERVABILITY.md` and `src/main.rs`.

### Q: Why no graceful shutdown / SIGTERM handler?

**Answer:** SlugAudit is a session-scoped child process, not a
daemon. It has exactly one caller (the MCP agent). When the caller
disconnects, stdin closes → the process exits. In-flight tool calls
are abandoned, but the caller is gone — there's nobody to return
results to. The database is disposable — WAL recovery or the
corruption path handles mid-publish kills on the next open.

Full rationale: "Process lifecycle" section above.

### Q: Why no HTTP health endpoint?

**Answer:** Stdio is the only transport. There's no HTTP listener to
hang a `/health` endpoint on. The MCP `health` tool provides the
same information through the existing transport. An HTTP health
endpoint belongs with SSE/HTTP transport, not stdio.

### Q: Why no connection pooling?

**Answer:** A fresh SQLite connection is opened per tool call. WAL
mode makes `open` cheap (no journal replay on every open). The
connection is the correctness boundary — read-only vs. read-write
mode is set at open time, and a connection's mode can't change.
Reusing connections would require tracking which connection is
which mode and handling corruption/poisoning per-connection.
Per-call open is simpler and correct.

### Q: Why no structured metrics / Prometheus endpoint?

**Answer:** The `health` MCP tool returns `AtomicU64` counters
(tool calls, errors, cumulative latency, consecutive full publishes,
watcher health, pending event counts). For the stdio deployment
model, pulling these via an MCP call is the right interface — the
same transport that calls tools can query health. A Prometheus
endpoint belongs with SSE/HTTP transport.

### Q: Why are findings session-scoped and purged on restart?

**Answer:** Findings are AI-authored conclusions from a specific
reasoning session, not durable facts about the code. A different
agent (different model, different chat) should not silently inherit
another session's conclusions — it would be tempted to treat them
as "already checked, skip." Evidence (files, hashes, dependency
edges) persists across sessions because it's reproducible from
source. Findings don't because they're not.

Full rationale: `ARCHITECTURE.md` invariant #9 and the 2026-08-12
DECISIONS.md entry "Findings scoped to the agent session that wrote
them."

### Q: Why is the ATTACH guard needed if the agent already has filesystem access?

**Answer:** It isn't a security boundary — it's an architectural
discipline. The invariant says "correctness comes from the
connection itself." `guard_against_attach` prevents future code
changes to the `query` tool from accidentally widening the SQL
surface to other databases. The agent could run `sqlite3` directly
to read any file it has permissions for; the ATTACH guard keeps
the invariant honest, not the user contained.

### Q: Why is progress notification delivery fire-and-forget (`tokio::spawn`)?

**Answer:** Progress notifications are best-effort — a broken
progress channel must never turn a successful tool call into an
error. Each notification spawns a tiny future (one RPC call).
Volume is throttled at 10 events/s. The alternative (a channel +
drain task) would need its own observable state for the same
best-effort guarantee with more complexity.

Full rationale: comments in `src/server_runner.rs::McpProgressSink`.

### Q: Which operating systems are supported?

**Answer:** SlugAudit supports Linux and macOS. The CI matrix and
filesystem-safety implementation cover those targets. Other operating
systems are rejected by the filesystem guard rather than running with
unknown SQLite safety properties.

### Q: Why does `record_error` lock through `lock_or_recover` instead of a raw mutex unwrap?

**Answer:** Consistency. The rest of the codebase uses the
`lock_or_recover` helper, which also logs at `error` on recovery, so
a poisoned mutex in the sample worker pool is recovered loudly
instead of silently. (`src/sync/sample_batch.rs` — an early version
of this FAQ documented a raw `.lock().unwrap_or_else()` that has
since been replaced.)

## Layering rules

Bottom-up: `store` and `parse` know nothing of MCP. `graph` knows
nothing of `sync`. `sync` knows nothing of `tools`. `tools` orchestrates
across all three but never duplicates their logic.

Cross-crate callers go through the following path:

- `sync::revision::publish_revision` is the single write path for every
  table. Nothing writes to `files`, `findings`, or `edges` directly.
- `tools::context::ensure_synced` is the single point where a project
  gets brought current. Every state-bearing tool calls it — `report`
  included: its snapshot is always built from a freshly ensured
  revision, never from a stale one.
- `tools::core::ToolCounters` is the single source for "how many tool
  calls have we served". Avoid adding a parallel counter outside this
  module — keep one definition.

## Testing layout

Tests live inline next to the code they cover, included via
`#[path = "..."  ] mod tests;`. The shape:

```
src/
└── module.rs                production code
                          └── #[cfg(test)] #[path = "module_tests.rs"] mod tests;
src/
└── module_tests.rs        (same directory, sibling)
```

The 54 `*_tests.rs` files share this pattern; `cargo test --lib` runs
all of them in parallel (`--test-threads=4` by default). Tool test
modules occasionally split a focused scenario into a second sibling
(e.g. `tools/finding_session_tests.rs`). Test files get a 500-code-line
ceiling — each `#[test]` names one behavior contract, repetition that
can't be DRY'd without losing failure diagnostics — while production
files keep the 300-line ceiling; the
`cargo run --bin check_source_limits` bin (formerly
`tools/check_source_limits.sh`) enforces both.

## Where to start reading

Joining the project? Read in this order:

1. **`.planning/PHASE-00.md`** — where the codebase came from and why.
2. **This file (ARCHITECTURE.md)** — overall structure. Pay particular
   attention to the **Design FAQ** (below) — it preemptively answers
   every "why didn't you do X?" question an auditor will ask — and the
   Security and trust model, Disposable-data philosophy, and Key
   invariants sections.
3. **`OBSERVABILITY.md`** — what telemetry exists and where it goes.
4. **`AUDIT.md`** — the most recent commercial audit with corrected
   findings and the current remediation roadmap.
5. **`src/main.rs`** + **`src/server.rs`** + **`src/server_runner.rs`** —
   the "what actually happens when a tool call arrives" story: tool
   contracts in `server.rs`, semaphore-bounded dispatch + progress in
   `server_runner.rs`.
6. **`src/sync/manager.rs`** — the synchronization state machine.
7. **`src/graph/resolver/`** — how imports become dependency edges.

If something in the codebase disagrees with this document, **fix one or
the other** — never both — and update any in-flight plan files to match.
