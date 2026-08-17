# SlugAudit Architecture

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

The server exposes six tools:

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
│   ├── project_control.rs    enable/disable
│   └── health.rs             operational snapshot (Phase 2.1)

├── sync/                     Indexing + watcher-aware reconciliation
│   ├── mod.rs                public exports
│   ├── manager.rs            SourceSyncManager: ensure_current, reconcile, health accessors
│   ├── manager_meta.rs       current_revision_id + ensure_project_row helpers
│   ├── reconcile.rs          dirty/deleted reconciliation + barrier cap
│   ├── discovery.rs          filesystem walk with extension/limit filters
│   ├── hash.rs               BLAKE3 content hashing
│   ├── sample.rs             read-file-with-budgets
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
└── cli_tests.rs etc.         inline #[path] test modules per source file
```

## File-size authorization summary

Every file under `src/` is governed by `cargo run --bin check_source_limits --locked` (the bin that replaced the prior `tools/check_source_limits.sh` shell script):
files at 0–199 code lines auto-pass the gate, 200–300 requires an
in-source comment of the form

```rust
// slugaudit-line-exception: approved-by=<who>; reason=<why>
```

and ≥300 hard-fails the gate (CI red). The four scopes below mirror
those stages as OAuth-style decisions so a reviewer can see at a
glance which files are explicit grants versus implicit auto-passes
versus active violations — without re-running the script.

| Stage | Scope decision | Count |
|---|---|---|
| 0–199 LoC | `source-size:auto` | 96 |
| 200–300 LoC, annotated | `source-size:approved-exception` | 9 |
| 200–300 LoC, **NOT** annotated | `source-size:violation` | 0 |
| ≥300 LoC | `source-size:hard-fail` | 0 |

### Approved exceptions — `source-size:approved-exception`

The following files exceed the 200 LoC soft cap with an
`approved-by=agent; reason=…` annotation justifying the bundling.

| File | LoC | Reason |
|---|---:|---|
| `src/graph/resolver/generic.rs` | 224 | core types + LanguageResolver trait dispatcher + GenericResolver impl are one cohesive runtime contract; per-language helpers live next to their resolvers |
| `src/graph/resolve_rust.rs` | 210 | one resolution pipeline per Rust import form (workspace crate anchoring, super/self module-tree walk, item-vs-module segment shortening) where every helper is mutually recursive on the same `known_paths` contract; splitting would force `pub(crate)` exports across files and duplicate the candidate-matching loop |
| `src/sync/manager.rs` | 290 | `ensure_current`'s three-branch match is the sync orchestrator hot path; trace sites + `stamp_last_sync` belong next to the code paths they cover |
| `src/sync/manager_tests.rs` | 259 | one end-to-end watcher-backed scenario per sync invariant, all sharing the same `create_project`/`write_file`/`sync_project` fixture helpers; splitting would force a cross-module test harness or duplicate the four helpers in every file |
| `src/sync/publish_tests.rs` | 255 | one end-to-end publish scenario per sync invariant, all sharing the `write`/`stored_paths` fixture helpers against a real SQLite database; splitting would force a cross-module test harness or duplicate the helpers in every file |
| `src/sync/reconcile.rs` | 201 | reconciliation is one atomic pipeline (snapshot + reconcile + manifest + publish); barrier-cap test belongs next to the loop it covers |
| `src/sync/reconcile_tests.rs` | 249 | one test per reconcile outcome (unchanged/modified/new/deleted/mixed/race/cap) sharing `setup_project` and `use super::*` access to `MAX_BARRIER_LOOPS` + `ReconcileError`; splitting would split the fixture from the loop constants it asserts against |
| `src/tools/health.rs` | 208 | health is the schema-defining tool; Request + Response + phase + derivation live together so the schema isn't split from its only consumer |
| `src/tools/query_tests.rs` | 201 | one test per SQL safety property; splitting would obscure the read-only boundary they collectively pin |

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

## Layering rules

Bottom-up: `store` and `parse` know nothing of MCP. `graph` knows
nothing of `sync`. `sync` knows nothing of `tools`. `tools` orchestrates
across all three but never duplicates their logic.

Cross-crate callers go through the following path:

- `sync::revision::publish_revision` is the single write path for every
  table. Nothing writes to `files`, `findings`, or `edges` directly.
- `tools::context::ensure_synced` is the single point where a project
  gets brought current. Every state-bearing tool calls it; reporting
  tools (`report`) bypass it for read-only freshness on the
  already-current revision.
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

The 51 `*_tests.rs` files share this pattern; `cargo test --lib` runs
all of them in parallel (`--test-threads=4` by default). Tool test
modules occasionally split a focused scenario into a second sibling
(e.g. `tools/finding_session_tests.rs`) so each file stays under the
200-code-line soft cap. Test files are subject to the same small-file
rule as production; the
`cargo run --bin check_source_limits` bin (formerly
`tools/check_source_limits.sh`) enforces both.

## Where to start reading

Joining the project? Read in this order:

1. **`.planning/PHASE-00.md`** — where the codebase came from and why.
2. **`OBSERVABILITY.md`** — what telemetry exists and where it goes.
3. **This file (ARCHITECTURE.md)** — overall structure.
4. **`src/main.rs`** + **`src/server.rs`** + **`src/server_runner.rs`** —
   the "what actually happens when a tool call arrives" story: tool
   contracts in `server.rs`, semaphore-bounded dispatch + progress in
   `server_runner.rs`.
5. **`src/sync/manager.rs`** — the synchronization state machine.
6. **`src/graph/resolver/`** — how imports become dependency edges.

If something in the codebase disagrees with this document, **fix one or
the other** — never both — and update any in-flight plan files to match.
