# Repository Guidelines

Guidance for AI coding agents and contributors working **on** SlugAudit — the tool
itself, not on a project that uses it.

## Project Overview

SlugAudit is a **stdio MCP server** that indexes a project into a disposable SQLite
database and answers *evidence* queries about it (parsed structure, symbols,
imports, diagnostics, session findings).

- SlugAudit **does not audit**: no risk detection, no severity, no conclusions, no
  recommendations. Every judgment belongs to the calling agent. Never describe
  SlugAudit as having identified, rated, or recommended anything.
- Shipped artifact = the single `slugaudit` binary. `tests/`, `benches/`,
  `src/bin/check_*`, `.github/`, and the repo's own `.planning/` docs are
  development-only (see `DEVELOPMENT_SCOPE.md`).
- Runtime data lives in `<project>/.planning/slugaudit/project.db` (+ sidecars):
  disposable derived data, excluded from discovery, gitignored via
  `/.planning/slugaudit/`. Findings are session-scoped and purged on first sync.
- Crate `slugaudit-mcp-rust`, version `1.0.10`, edition 2024, `publish = false`,
  `license-file = LICENSE` (non-SPDX PolyForm Noncommercial 1.0.0 + tool-use
  permission).

## Architecture & Data Flow

**`.planning/ARCHITECTURE.md` is the authoritative source of truth** — module map,
data flow, process lifecycle, and the 11 key invariants. Read it before any
structural change; this file is a summary, not a replacement.

### Request path (one tool call)

`src/main.rs` → `SlugAuditServer` (rmcp `#[tool_router]`, `src/server.rs`) →
`dispatch` → `server_runner::run_blocking` → `tools::context::ensure_synced` →
`SourceSyncManager::ensure_current` → reconcile/publish → `with_verified_read` or
`with_verified_write` → `Json` response.

- `src/server.rs` declares *contracts only* (schemas, `INSTRUCTIONS`, thin
  handlers). Execution mechanics — semaphore, `spawn_blocking`, MCP progress
  notifications — live in `src/server_runner.rs`. Keep that split.
- **Streaming publishing is forbidden under `serve`**: stdout *is* the JSON-RPC
  transport, so any `println!`/`print!` in serve-reachable code is a hard lint error.
- Progress: `ProgressEvent::{Started, Sampling, Completed}` flows sync →
  `ProgressSink` → server_runner → MCP `notify_progress` (`0.0`/`0.5`/`1.0`, plus
  throttled per-file sampling). Sends are best-effort; a broken channel never fails
  a successful call.

### Indexing and revision model

- `sync/manager_orchestration.rs::ensure_current` = resolve project → open or
  rebuild → verify metadata → reconcile/rebuild → return
  `SyncedProject { database_path, revision_id }`.
- `sync/manager_health.rs::sync_by_health` branches on `WatcherHealth`:
  `NeedsVerification`/`Desynced` → full publish + drain; `Healthy` → reconcile +
  stat sweep; `Unavailable` → full publish every call.
- Every publish is compare-and-swap: `publish_cas.rs` retries only
  `StaleBaseline`/`ChangedDuringSample` up to `MAX_CAS_RETRIES = 4` under one shared
  `Deadline`; `sync/revision.rs::publish_revision` is the **only** write path for
  `files`/`evidence`/`dependency_edges`/`revisions`, and flips `is_current` last.
- `revision_id` is rowid-derived (`rev-{id}`), not hash-derived, and changes on
  every publish. `tools/context_transactions.rs` re-verifies it inside the
  transaction and the caller retries — a successful `ensure_current` never
  guarantees no concurrent publish.
- Freshness has two layers: the `notify` watcher (which *drops* events when the
  sync thread holds the lock, via `try_lock`) and the watcher-independent stat sweep
  (`sync/sweep.rs`). Treat the sweep as the guarantee, the watcher as an
  optimization. A same-size edit inside the same second with no watcher event is a
  documented blind spot.

### Concurrency

- Blocking tool work runs on Tokio's blocking pool gated by one
  `Arc<Semaphore>` of `MAX_CONCURRENT_BLOCKING_OPS = 8` per server; the permit is
  moved *into* the blocking closure so cancellation can't free it early.
- Barrier sync loops at most `MAX_BARRIER_LOOPS = 16` times before marking the
  project `Desynced`.
- Per-server state, not globals: `SourceSyncManager` is injected via
  `SlugAuditServer::with_manager` (tests pass `SourceSyncManager::default()`, which
  starts **no** watcher). Process-global state is limited to `ToolCounters`
  atomics, `util::SESSION_ID`, `model::limits::process_limits` (`OnceLock`), and
  `parse::PACK_VERSION`.
- Every `Mutex` lock goes through `util::lock_or_recover` (poison recovery, never a
  re-panic).

### Key invariants (`.planning/ARCHITECTURE.md`, 11 total)

1. Correctness comes from the connection, never from query text.
2. Symlinks and network filesystems are rejected at open time.
3. All `Mutex` locks go through `lock_or_recover`.
4. Barrier sync is capped at `MAX_BARRIER_LOOPS = 16`.
5. Tools use `run_blocking` gated by the 8-permit semaphore.
6. Each revision is published atomically (CAS).
7. Findings auto-invalidate on file change.
8. `#![forbid(unsafe_code)]` crate-wide.
9. Findings are scoped to the writing session (`purge_prior_session_findings`).
10. The database is disposable derived data.
11. Production code never panics on an error value.

### Security and trust model

The threat model is a **lazy/unsupervised AI agent, not a hostile attacker** — the
agent already has filesystem access to the same directory as the DB.

- Security here means convenience guards, not defense against a malicious actor.
  The stdio transport is the auth boundary: no network listener, no multi-client
  multiplexing, no auth.
- The `query` tool's read-only connection + subquery wrapper + authorizer hook are
  the real guards; the SQL keyword blocklist is defense-in-depth.
- `structure` executes agent-supplied Tree-sitter queries — the most interesting
  surface, because the agent can ask SlugAudit to do work the human didn't request.
- `finding` stores agent-authored text verbatim by design: the human reads it
  directly, and session-scoped purge prevents cross-contamination.
- The DB is deliberately throwaway so past agents can't leave stale "this is
  unfinished" commentary that misleads the next one.

## Key Directories

| Path | Purpose |
|---|---|
| `src/server.rs`, `src/server_runner.rs` | MCP surface: tool contracts, dispatch, blocking pool, progress |
| `src/tools/` | Tool handlers: `report`, `query`, `structure`, `leads`, `finding`, `finding_read`, `project_control`, `health` |
| `src/sync/` | Indexing: manager, orchestration, health, reconcile, publish/CAS, revision, sweep, discovery, manifest |
| `src/store/` | The only SQLite boundary: connection guards, migrations, `schema.sql` (`CURRENT_SCHEMA_VERSION = 3`) |
| `src/watch/` | `notify` watcher, per-project `WatchState`, `WatcherHealth` |
| `src/evidence/` | Tree-sitter evidence extraction + normalization (`extract`, `to_row`) |
| `src/graph/` | Import resolution (Rust/Python/JS resolvers, `resolve_imports`) |
| `src/parse/`, `src/model/` | Language detection + `PACK_VERSION`; limits, profiles, spans, source models |
| `src/project/` | Project root resolution, activation marker, database path, `config.json` |
| `src/bin/` | Development gate binaries (see Development Commands) |
| `tests/` | Integration crates: `stdio_protocol`, `menu_tests`, `connect_tests` |
| `benches/` | Criterion targets: discovery, parsing, search, startup, memory |
| `examples/queries/` | Query cookbook **and** the `include_str!` source of `slugaudit://` resources |
| `.planning/` | `ARCHITECTURE.md` (authoritative) + `archive/` (completed plans, DECISIONS, perf baseline) |
| `.planning/slugaudit/` | Runtime DB state, gitignored — never documentation, never committed |
| `docs/` | Per-MCP-client setup guides (`claude-code.md`, `codex.md`, `bob.md`, `grok.md`) |

## Development Commands

```bash
cargo build --release --locked                    # the shipped binary
cargo run                                          # serve MCP over stdio (default-run = slugaudit)
cargo fmt --all -- --check
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --bins --tests --all-features --locked   # CI bulk run
cargo test --lib <name_substring>                         # one test
cargo test --lib -- tools::leads::tests                   # one module
cargo test --test stdio_protocol --locked                 # integration crate
cargo bench --bench discovery                             # one bench
```

Quality gates are pure-Rust cargo bins (no shell/Python), run in this order in CI:

```bash
cargo run --quiet --bin check_source_limits --locked   # LoC ceilings + exception annotations
cargo run --quiet --bin check_no_duplicates --locked   # duplicate commit subjects / test fn names
cargo run --quiet --bin check_docs_drift --locked      # ARCHITECTURE.md module map + test-file count
cargo run --quiet --bin check_coverage --locked        # llvm-cov line gate, default 83%
cargo run --bin check_performance --locked             # criterion medians vs baseline, default 20%
cargo run --bin check_performance -- --record          # deliberately re-baseline perf
```

CI also runs `cargo-deny` (`check advisories bans sources licenses`), `rustsec/audit-check`,
and, on Linux only, `cargo mutants` over `src/sync/revision.rs`, `src/sync/publish.rs`,
`src/sync/hash.rs`, `src/tools/context.rs` (fails closed).

Local helpers: `bash scripts/fast-check.sh <file.rs>` (pre-check only — it is **not**
in CI and its thresholds/annotation parsing differ from `check_source_limits`);
`bash install.sh` (installs to `${HOME}/.slugthug/slugaudit`); `slugaudit update`.

## Code Conventions & Common Patterns

### Lint contract (`Cargo.toml` + `src/lib.rs`, applies to all targets)

- `unsafe_code = "forbid"`, `unreachable_pub`/`trivial_casts`/`trivial_numeric_casts`/`elided_lifetimes_in_paths = "deny"`.
- `clippy::print_stdout = "deny"` — stdout is the JSON-RPC transport. CLI-facing
  modules (`main.rs`, `cli.rs`, `menu*.rs`, `install.rs`, `update.rs`, `connect.rs`)
  and gate bins carry a file-level `#![allow(clippy::print_stdout)]`.
- `clippy::todo`, `clippy::unimplemented`, `clippy::exit`, `clippy::clone_on_ref_ptr` denied.
- `clippy::unwrap_used` / `clippy::expect_used` denied **outside `cfg(test)`**
  (`src/lib.rs`, repeated in every gate bin). Tests are exempt: there a panic *is*
  the failure signal.

### Production code must not panic

Fallible work returns `Result`; a genuinely fatal condition (failed process startup)
reports the error and picks an exit code. A panicking blocking task is caught in
`server_runner.rs`: `spawn_blocking(..).await.map_err(|e| ErrorData::internal_error(..))`
turns a `JoinError` into an error response, never a crash.

### Error handling

- Tool boundaries return `Result<_, rmcp::ErrorData>`.
- Contract/user errors → `ErrorData::invalid_params` (unknown mechanic, oversized
  SQL, unindexed file, inactive project).
- Operational/transient errors → `tools::context::internal_error(context, error)`,
  whose message tells the caller the failure is often transient and to retry.
  Sibling translators: `tools::query::describe_error`, `tools::project_control::err`.
- Corruption and staleness are **recovered, not surfaced**: `open_or_rebuild` /
  `verify_metadata_or_rebuild` discard and rebuild the DB on corruption or version
  mismatch.

### Adding or extending a tool

1. Add `src/tools/<name>.rs` with `Request`/`Response` types (`serde` +
   `schemars::JsonSchema`; field docs become JSON-Schema descriptions).
2. Register it in `src/tools/mod.rs` (`mod <name>; pub use <name>::{..}`) and add
   the `#[tool] async fn` handler plus a parameters struct in `src/server.rs`.
3. Open data through `ensure_synced` + `with_verified_read`/`with_verified_write`.
4. Follow the `leads` shape: module-level consts for caps, complete
   `mechanic_counts` regardless of paging, rows capped (mirrors `query`'s
   `MAX_ROWS = 500`), `detail` truncated on a char boundary, `next_offset` only when
   more remain, and a `tracing::info!` completion log.
5. Adding a `leads` mechanic is **one new entry** in the `MECHANICS` table
   (`src/tools/leads_query.rs`) — never a new table. Shared predicates such as
   `CREDENTIAL_PATH_PREDICATE` live in `tools/mod.rs` so surfaces can't disagree.

### Source-size gate

Code-line ceilings (token-aware counting; comments/strings excluded):
**200** production default · **200–300** allowed only with
`// slugaudit-line-exception: approved-by=slugthug; reason=<non-empty>` ·
**301–350** only with a dated human approval
`// slugaudit-line-exception: approved-by=human-user; date=YYYY-MM-DD; reason=<non-empty>` ·
**>300** or **>500 for tests** hard-fails. Only `*_tests.rs` and `tests.rs` count as
test files — `test_support.rs`, `proptest.rs`, `race_hook.rs` fall under the 200-line
production cap.

### Doc-drift coupling (`check_docs_drift`)

- Renaming/adding/removing anything under `src/` requires updating the fenced
  module-map block in `.planning/ARCHITECTURE.md` (every listed path must exist).
- Adding/removing a `src/**/*_tests.rs` file requires updating the
  “The 59 `*_tests.rs` files share this pattern” sentence (currently 59).
- `.planning/archive/IMPLEMENTATION_PLAN.md` must keep a `§22` Status reference, and
  any descoped task section needs a matching `DECISIONS.md` entry.
- Doc lookup order is `root/` → `.planning/` → `.planning/archive/` (first match
  wins), so gate-consumed docs must live in one of those three roots.
- Active planning docs go in `.planning/`; completed records go in
  `.planning/archive/`.

## Important Files

| File | Why it matters |
|---|---|
| `src/main.rs`, `src/cli.rs` | Binary entry, arg parsing, stderr-only tracing (`SLUGAUDIT_LOG_FORMAT=json`) |
| `src/server.rs` | Every tool schema, the `INSTRUCTIONS` briefing, `with_manager`, server info |
| `src/server_runner.rs` | `run_blocking`, `MAX_CONCURRENT_BLOCKING_OPS = 8`, progress throttling |
| `src/tools/context.rs`, `context_transactions.rs` | `ensure_synced`, revision verification, read/write transactions |
| `src/sync/revision.rs`, `publish_cas.rs`, `manager_health.rs` | The atomic publish path and health state machine |
| `src/store/schema.sql`, `store/migrations.rs` | Index schema and `CURRENT_SCHEMA_VERSION` |
| `src/ignore_rules.rs` | Single source of truth for exclusions (discovery, watcher, reconcile) |
| `src/resources.rs` + `examples/queries/` | `slugaudit://` resources, compiled in via `include_str!` |
| `.planning/ARCHITECTURE.md` | Authoritative design + invariants (gate-checked structure) |
| `.planning/archive/` | DECISIONS, IMPLEMENTATION_PLAN, perf baseline, phase records |
| `Cargo.toml` | Bins, benches, lint policy, dependency pins |
| `rust-toolchain.toml`, `clippy.toml`, `deny.toml` | Pinned toolchain, doc idents, supply-chain policy |
| `tests/stdio_protocol.rs` | End-to-end MCP handshake over a real child process |

## Runtime/Tooling Preferences

- **Rust 1.97.1** pinned in `rust-toolchain.toml` (rustfmt + clippy components);
  edition 2024. There is **no declared MSRV** — do not invent one.
- Dependencies: 16 runtime + 4 dev. `tree-sitter-language-pack = "=1.14.3"` is the
  only exact pin; `src/parse/PACK_VERSION` is a hand-duplicated string that a test
  asserts against the manifest pin — update both together.
- Supply chain (`deny.toml`): no advisories ignored, no wildcards, no duplicate
  versions outside a reviewed skip list, sources restricted to crates.io, licenses
  allow-listed (Apache-2.0, BSD-2/3-Clause, CC0-1.0, CDLA-Permissive-2.0, ISC, MIT,
  MPL-2.0, Unicode-3.0, Zlib). A new dependency must clear `cargo deny`.
- Logs go to **stderr** only; `SLUGAUDIT_LOG_FORMAT=json` switches the subscriber to
  JSON.
- Limits resolve in two layers: `SLUGAUDIT_PROFILE=adaptive|audit|lean` plus
  `SLUGAUDIT_<FIELD>` overrides (cached in a `OnceLock`), then
  `.planning/slugaudit/config.json` via `ProjectConfig::apply_to_limits` — where a
  project-file `profile` **replaces** the whole env-derived struct.
- Releases are Linux x86_64 only (`release.yml`), shipped with `SHA256SUMS`;
  `install.sh`/`slugaudit update` verify the checksum before installing.
- Prefer SlugAudit's **own MCP tools** when inspecting this repo: `report` for shape,
  `query` (read-only SQL over `files`/`evidence`/`dependency_edges`) instead of
  disk reads, `structure` for Tree-sitter pattern matches, and `leads` for
  mechanical counts. Join `evidence` to `files` on `file_id`.
- Stale docs exist — don't be misled: `ARCHITECTURE.md` references
  `OBSERVABILITY.md`, `PACKAGING.md`, `AUDIT.md`, and `.planning/PHASE-*.md`, none of
  which exist (phases live in `.planning/archive/plan-audit/`); README/docs quote an
  89% coverage threshold while the gate default is 83%; the exception-annotation
  examples there use `approved-by=<who>`/`approved-by=agent`, which the gate
  **rejects**.

## Testing & QA

### Layout

- Suites live beside their subject. Large ones are sibling files pulled in at the
  bottom of the production file:
  ```rust
  #[cfg(test)]
  #[path = "leads_tests.rs"]
  mod tests;
  ```
  Small modules keep an inline `#[cfg(test)] mod tests { use super::*; .. }`.
  Extra siblings append the concern (`publish_race_tests.rs`,
  `context_transaction_tests.rs`).
- Shared fixtures: `src/sync/test_support.rs` (largest — `create_project`,
  `setup_project`, `sync_project`, `stored_paths`), `src/tools/test_support.rs`
  (`activated_project`), `src/graph/test_support.rs`, and
  `src/bin/check_docs_drift/test_support.rs`.
- Integration tests are separate crates in `tests/`; benches are criterion binaries
  with `harness = false`.

### House style

- One `#[test]` per behavior, named for the observable contract in snake_case
  (`counts_every_mechanic_without_returning_rows`), often opening `a_`/`an_`/`the_`.
- Arrange / blank line / act / blank line / assert; every file starts with `use super::*;`.
- Failures are panics: `.expect("short reason")`, assertions carry a diagnostic.
  Tests return `()`, not `Result`. `#[tokio::test]` for async.
- Assert behavior, boundaries, invariants, transitions, and real errors — never
  wiring, field copies, or source text.
- Test function names must be **globally unique** across `src/` and `tests/`
  (`check_no_duplicates`), as must commit subjects.

### Fixtures and shared state

- `tempfile::tempdir().expect(..)`, held for the test's lifetime.
- `SourceSyncManager::default()` / `::new()` starts **no** watcher (use it by
  default); `with_watcher()` starts the real `notify` watcher and needs debounce
  waits.
- `NoopProgressSink` is the default sink for every tool/publish call.
- Env mutation: `temp_env::with_var*` **plus** `util::TEST_ENV_LOCK`; tests touching
  `SESSION_ID` take `util::SESSION_TEST_LOCK`; `ToolCounters` are process-wide and
  cannot be reset (assert deltas); `process_limits()` is a `OnceLock`, so env changes
  after first use are ignored.
- `src/sync/race_hook.rs` is the deterministic no-sleep race barrier — `cfg(test)`
  and private to `sync`; `tools` tests roll their own `mpsc` barrier.
- `tests/connect_tests.rs` mutates real agent config and self-skips without an agent
  CLI on PATH; `tests/stdio_protocol.rs` spawns the built binary with 10 s receive
  timeouts (load-sensitive).

### Expectations

- Coverage: `cargo run --quiet --bin check_coverage --locked`, default gate **83%**
  line coverage via `cargo llvm-cov` (requires `cargo-llvm-cov`).
- Performance: 20% median regression allowance against
  `.planning/archive/perf_baseline.json`; startup and memory benches are measured but
  **not** gated. `check_performance` is not currently wired into CI — run it locally
  when touching the discovery/parsing/search paths.
- Property tests: only `src/graph/resolver/proptest.rs`; its seed corpus in
  `proptest-regressions/` is committed — never delete it.
