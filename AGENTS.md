# Contributing to SlugAudit

Guidance for AI coding agents and contributors working **on** SlugAudit — the
tool itself, not on a project that uses it.

---

## Why this exists

The product was founded on one observation, and that observation is the only
success criterion this project has ever had:

> A large audit was underway. SQLite was suggested as a way to stop re-reading
> files. The agent tried it, then **kept calling it — call after call,
> unprompted — and afterward said, in its own words, why it helped.** Sessions
> where that happened also **found issues that had been missed** and **finished
> audits faster with the extra results**.

That is the target: **an agent that lights up the screen and says why.**
Voluntary repetition inside one session, triggered by live pain, confirmed
afterward. Token savings are a byproduct. Tone is not the product. **Not missing
an issue is the product.**

This metric was never written down in the repository, so each past author
optimized a proxy it invented for itself — audience framing, page caps, honesty
in the pitch — which is how the surface drifted away from the thing that worked.
It is written down here so the next author has the real target in front of it.

### Accuracy is the goal

A **wrong** result costs more than a missing one. A count that needs a caveat
the response doesn't carry is a defect, not a rounding error: the agent quotes
it, builds a finding on it, and the finding is fabricated. Honesty belongs in
the output — a cap, a skip, or a producer gap is reported in the response or it
is a bug — never in the pitch. Do not add self-deprecating hedging to
marketing-facing prose to compensate for a response that should have carried the
truth itself.

### Non-negotiables (do not trade these away)

1. **Evidence is not judgment.** No risk scores, no severity, no ranking, no
   "priorities". Findings remain agent-authored. This is what makes every number
   quotable; a count carrying a verdict becomes arguable and therefore useless.
2. **Freshness without re-verification.** Sync before answering stays automatic.
   An index an agent must double-check has no value.
3. **No silent truncation, anywhere.** Every cap, skip, and producer gap is
   either reported in the response or it is a defect.
4. **Every count must be quotable as-is.** If a number needs a caveat the
   response doesn't carry, it is wrong.
5. **No new surface may become the sole holder of a fact.** The database stays
   disposable.

### Guiding design rule

An agent handed a new tool is not neutral; the rational prior is *"another one
that does less than what I already have."* That prior is broken by exactly one
thing: **call #1 returning something true the agent did not ask for and could
not have cheaply learned.** A first response that returns exactly what was
requested confirms the prior.

### Anti-goals

- Restoring `risk_score`, severity enums, ranked leads, or any ranked brief.
  Ranking is what makes output arguable, and arguable output is what sends an
  agent back to grep.
- Vanity copy — testimonials, self-praise, or a rave placed in the tool's own
  docs. Praise from inside the documentation is worth zero and is gameable.
- Optimizing for sentiment. A rave is cheap and sycophantic; a call sequence is
  not.
- Turning a row cap into a bigger row cap. The unit is bytes.

### Acceptance (the only metric that counts)

Over real audits: agents make **chains** (`report`/`leads` → `query`/`structure`
→ `finding`), not 2–3 calls then grep; sessions where the tool is used report
**extra findings** that grep-and-read missed; sessions finish faster *with* the
extra results. If agents still make a few calls and return to grep, say so.

---

## Session hygiene: if it isn't written down, it didn't happen

The next agent reads this repository, not the previous conversation. A transcript, a chat
log, and an agent's own working memory are all invisible to it — so anything decided,
discovered, or rejected that exists only in conversation is lost the moment the session
ends, and the next agent is not left arguing with it, it is left guessing. Before a session
ends, answer this and then act on the answer:

> **What did we decide, discover, or reject this session that is not yet written down?**

1. **Write it in the same turn you learn it** — not "afterwards". The sessions that end
   abruptly are exactly the ones that mattered, and a finding held in working memory is a
   finding lost.
2. **Put it where someone will look.** Bugs and gaps → `.planning/PLAN.md` (a phase, or the
   correctness-follow-ups list). Why a choice was made → `.planning/archive/DECISIONS.md` or
   the plan's decisions section. How an external thing behaves → the code comment *next to
   what it constrains*, carrying the version and date it was verified against.
3. **Give every item a state**: `landed`, `not started`, `not scheduled`, `resolved`,
   `still open`, `dropped because …`. An unstated item reads as "unknown" to the next agent,
   which is worse than "known bad".
4. **Never delete an unresolved item.** Close it with evidence, or drop it with a reason;
   silently removing it means it gets rediscovered from scratch.
5. **Prefer the verifiable form** — a date, a version, a path, a command, a `path:line` — so
   a claim can be re-checked instead of re-argued. Unverifiable claims rot quietly: the
   YAML-crate note in `src/connect_agents.rs` had to be corrected three times before it was
   anchored to release dates, and each earlier version looked just as convincing.

## What SlugAudit is

A **stdio MCP server** that indexes a project into a disposable SQLite database
and answers *evidence* queries about it — parsed structure, symbols, imports,
diagnostics, session findings.

- SlugAudit **does not audit**: no risk detection, no severity, no conclusions,
  no recommendations. Every judgment belongs to the calling agent. Never
  describe SlugAudit as having identified, rated, or recommended anything.
- Crate `slugaudit-mcp-rust`, version `1.0.10`, edition 2024, `publish = false`,
  `license-file = LICENSE` (non-SPDX PolyForm Noncommercial 1.0.0 plus the
  tool-use additional permission). `Cargo.toml`.
- Runtime data lives in `<project>/.planning/slugaudit/project.db` (+ sidecars):
  disposable derived data, excluded from discovery, gitignored via
  `/.planning/slugaudit/` (`.gitignore`). Findings are session-scoped and prior
  sessions' rows are purged on the first sync
  (`src/sync/manager_meta.rs::purge_prior_session_findings`).

`src/server.rs`'s `INSTRUCTIONS` constant is the primary usage doc and ships
inside the binary; keep it and this file consistent when the surface changes.

## Shipped artifact vs development-only

The end-user product is the single `slugaudit` binary. In this repository, the
following are development-only and are not shipped in releases:

- `tests/`, `benches/`
- `src/bin/check_*` (quality and CI tools; not required by an agent using the
  product)
- `.github/` workflows
- this repository's own `.planning/` documentation

When auditing this repository: score `slugaudit` runtime behavior as product
functionality; treat `<customer-project>/.planning/slugaudit/` as runtime
application state and the rest of a customer's `.planning/` as customer data;
do not count development-only check binaries as end-user runtime code; and do
not treat this repository's internal planning documents as packaged runtime
code. Distinguish CI/release-process failures from runtime failures.

Supported runtime targets are **Linux and macOS**; other platforms fail closed
at database open rather than silently accepting an unknown mount
(`src/store/netfs.rs`).

---

## Architecture and data flow

**`.planning/ARCHITECTURE.md` is the authoritative source of truth** — module
map, data flow, process lifecycle, security model, and the numbered invariants.
Read it before any structural change. This section is a summary, not a
replacement.

### Request path (one tool call)

`src/main.rs` → `SlugAuditServer` (rmcp `#[tool_router]`, `src/server.rs`) →
`dispatch` (`src/server.rs`) → `server_runner::run_blocking` →
`tools::context::ensure_synced` → `SourceSyncManager::ensure_current` →
reconcile/publish → `with_verified_read` or `with_verified_write` → `Json`
response.

- `src/server.rs` declares *contracts only* (schemas, `INSTRUCTIONS`, thin
  handlers). Execution mechanics — the semaphore, `spawn_blocking`, MCP
  progress notifications — live in `src/server_runner.rs`. Keep that split.
- **Streaming publishing is forbidden under `serve`**: stdout *is* the JSON-RPC
  transport, so any `println!`/`print!` in serve-reachable code is a hard lint
  error.
- Progress: `ProgressEvent::{Started, Sampling, Completed}` flows from the sync
  layer → `ProgressSink` → `server_runner` → MCP `notify_progress` (`0.0`
  queued, `0.5` publishing, `1.0` completed, plus throttled per-file
  `Sampling`). Sends are best-effort; a broken channel never fails a successful
  call (`src/server_runner.rs`, `src/progress.rs`).

### Indexing and revision model

- `src/sync/manager_orchestration.rs::ensure_current` resolves the project →
  opens or rebuilds the database → verifies metadata → reconciles/rebuilds →
  returns `SyncedProject { database_path, revision_id }`.
- `src/sync/manager_health.rs::sync_by_health` branches on `WatcherHealth`:
  `NeedsVerification`/`Desynced` → full publish + drain; `Healthy` → one
  reconcile + stat sweep; `Unavailable` → full publish every call.
- Every publish is compare-and-swap: `src/sync/publish_cas.rs` retries only
  `StaleBaseline`/`ChangedDuringSample` up to `MAX_CAS_RETRIES = 4` under one
  shared `Deadline`. `src/sync/revision.rs::publish_revision` is the **only**
  write path for `files`/`evidence`/`dependency_edges`/`revisions`, and flips
  `is_current` last.
- `revision_id` is rowid-derived (`rev-{id}`), not hash-derived, and changes on
  every publish. `src/tools/context_transactions.rs` re-verifies it inside the
  transaction and the caller retries — a successful `ensure_current` never
  guarantees no concurrent publish.
- Freshness has two layers: the `notify` watcher (which *drops* events when the
  sync thread holds the lock, via `try_lock`, `src/watch/manager.rs`) and the
  watcher-independent stat sweep (`src/sync/sweep.rs`). Treat the sweep as the
  guarantee, the watcher as an optimization. A same-size edit inside the same
  second with no watcher event is a documented blind spot.

### Concurrency

- Blocking tool work runs on Tokio's blocking pool gated by one
  `Arc<Semaphore>` of `MAX_CONCURRENT_BLOCKING_OPS = 8` per server; the permit
  is moved *into* the blocking closure so cancellation can't free it early
  (`src/server_runner.rs`).
- Barrier sync loops at most `MAX_BARRIER_LOOPS = 16` times before marking the
  project `Desynced` (`src/sync/reconcile/mod.rs`, `reconcile/barrier.rs`).
- Per-server state, not globals: `SourceSyncManager` is injected via
  `SlugAuditServer::with_manager` (tests pass `SourceSyncManager::default()`,
  which starts **no** watcher). Process-global state is limited to the
  `ToolCounters` atomics (`src/tools/mod.rs`), `util::SESSION_ID`,
  `model::limits::process_limits` (a `OnceLock`), and `parse::PACK_VERSION`.
- Every `Mutex` lock goes through `util::lock_or_recover` (poison recovery,
  never a re-panic; `src/util.rs`).

### Invariants every contributor must not break

These are enforced in code (see `ARCHITECTURE.md` for the full numbered
list):

- Correctness comes from the connection, never from query text: `query` opens
  `SQLITE_OPEN_READ_ONLY` and installs an authorizer that denies
  `ATTACH`/`DETACH`; there is **no** SQL keyword blocklist or table allowlist
  (`src/store/connection.rs`, `src/tools/query.rs`).
- Symlinks and network filesystems are rejected at open time
  (`src/store/connection.rs`).
- `#![forbid(unsafe_code)]` crate-wide (`src/lib.rs`).
- Production code never panics on an error value; all `Mutex` locks recover.
- Each revision is published atomically (CAS); the database is disposable
  derived data; findings are scoped to the writing session.

### Security and trust model

The threat model is a **lazy/unsupervised AI agent, not a hostile attacker** —
the agent already has filesystem access to the same directory as the database.

- Security here means convenience guards, not defense against a malicious
  actor. The stdio transport is the auth boundary: no network listener, no
  multi-client multiplexing, no auth.
- The `query` tool's read-only connection, subquery wrapper, and authorizer hook
  are the real guards.
- `structure` executes agent-supplied Tree-sitter queries — the most interesting
  surface, because the agent can ask SlugAudit to do work the human didn't
  request.
- `finding` stores agent-authored text verbatim by design: the human reads it
  directly, and session-scoped purge prevents cross-contamination.
- The database is deliberately throwaway so a past agent cannot leave stale
  "this is unfinished" commentary that misleads the next one.

---

## Code conventions

### Lint contract (`Cargo.toml` + `src/lib.rs`, applies to all targets)

- `unsafe_code = "forbid"`;
  `unreachable_pub`/`trivial_casts`/`trivial_numeric_casts`/`elided_lifetimes_in_paths = "deny"`.
- `clippy::print_stdout = "deny"` — stdout is the JSON-RPC transport. CLI-facing
  modules whose stdout writes happen before serve mode (and every gate bin)
  carry a file-level `#![allow(clippy::print_stdout)]`.
- `clippy::todo`, `clippy::unimplemented`, `clippy::exit`,
  `clippy::clone_on_ref_ptr` denied.
- `clippy::unwrap_used` / `clippy::expect_used` denied **outside `cfg(test)`**
  (`src/lib.rs`, repeated in every gate bin). Tests are exempt: there a panic
  *is* the failure signal.

### Production code must not panic

Fallible work returns `Result`; a genuinely fatal condition (failed process
startup) reports the error and picks an exit code. A panicking blocking task is
caught in `src/server_runner.rs`: `spawn_blocking(..)`'s `JoinError` becomes an
`ErrorData` response, never a crash.

### Error handling

- Tool boundaries return `Result<_, rmcp::ErrorData>`.
- Contract/user errors → `ErrorData::invalid_params` (unknown mechanic,
  oversized SQL, unindexed file, inactive project, malformed structure query).
- Operational/transient errors → `tools::context::internal_error(context, error)`,
  whose message tells the caller the failure is often transient and to retry.
  Sibling translators: `tools::query::describe_error`,
  `tools::project_control::err`.
- A failing call may carry a **machine-readable code**: `ErrorData.data` is
  `{"code": …, "remedy": …}`, drawn from the closed set in
  `src/error_codes.rs` and documented in `.planning/ARCHITECTURE.md` (a unit
  test checks the two in both directions). The sync budget is the case that
  forced it — a spent `max_sync_wall_clock` is permanent on a repo too big for
  the limit, so its remedy names the knob rather than advising the retry the
  generic message suggests. Remedies are static: never interpolate paths,
  SQL, or file content into a code or a remedy. Attach a code where the typed
  failure exists (discovery/publish/reconcile/sweep/activation), not by
  matching a message at the outermost tool.
- Corruption and staleness are **recovered, not surfaced**:
  `open_or_rebuild` / `verify_metadata_or_rebuild`
  (`src/sync/manager_orchestration.rs`) discard and rebuild the database on
  corruption or a schema/contract version mismatch (mismatch needs no
  migration code).

### Adding or extending a tool

1. Add `src/tools/<name>.rs` with `Request`/`Response` types (`serde` +
   `schemars::JsonSchema`; field docs become JSON-Schema descriptions).
2. Register it in `src/tools/mod.rs` and add the `#[tool] async fn` handler plus
   a parameters struct in `src/server.rs`. Give the handler a `title` and
   `annotations` (`read_only_hint`, `destructive_hint`) — the return type's
   `JsonSchema` is the MCP `outputSchema` automatically — then re-record the
   surface snapshot (`cargo run --bin check_mcp_surface -- --record`), which is
   what makes a surface change a deliberate, reviewable one.
3. Open data through `ensure_synced` + `with_verified_read`/`with_verified_write`.
4. Follow the `leads` shape: module-level consts for caps, complete
   `mechanic_counts` regardless of paging, rows capped (mirrors `query`'s
   `MAX_ROWS = 500`), `detail` clipped on a char boundary and flagged with
   `detail_clipped`, `next_offset` only when more remain, and a completion
   `tracing::info!`.
5. Adding a `leads` mechanic is **one new entry** in the `MECHANICS` table
   (`src/tools/leads_query.rs`) — never a new table, and never a statement about
   what a match means. Shared predicates such as `CREDENTIAL_PATH_PREDICATE`
   live in `src/tools/mod.rs` so surfaces can't disagree.

### Source-size gate (`check_source_limits`)

Code-line ceilings under `src/`. Counting is token-aware: a line counts only
when it holds a non-comment, non-literal Rust token, so comments, string and
char literals, and raw-string literals are excluded
(`src/bin/check_source_limits/counter.rs`):

| Code lines | Rule |
|---|---|
| 0–199 | auto-pass |
| 200–300 | needs `// slugaudit-line-exception: approved-by=slugthug; reason=<non-empty>` |
| 301–350 | needs `// slugaudit-line-exception: approved-by=human-user; date=YYYY-MM-DD; reason=<non-empty>` |
| >350 | hard fail — no exception rescues a file past the production ceiling |
| test files | auto-pass up to 500; >500 hard fail, no exception path |

Only `*_tests.rs` and `tests.rs` count as test files (`approval.rs`,
`counter.rs`); `test_support.rs`, `proptest.rs`, `race_hook.rs` fall under the
production cap. The annotation parser accepts only the two `approved-by=` forms
above — an unrecognized `approved-by=` value is rejected, not ignored.

### Doc-drift coupling (`check_docs_drift`)

The gate enforces three things:

- Renaming/adding/removing anything under `src/` requires updating the fenced
  module-map block in `.planning/ARCHITECTURE.md` (every listed path must
  exist).
- Adding/removing a `src/**/*_tests.rs` file requires updating the
  “N `*_tests.rs` files share this pattern” sentence in `.planning/ARCHITECTURE.md`
  (the gate compares the claimed number against the real count).
- The canonical doc set exists: `README.md`, `AGENTS.md`,
  `.planning/ARCHITECTURE.md`, `.planning/PLAN.md`,
  `.planning/archive/DECISIONS.md`, `docs/CLIENTS.md`,
  `docs/TROUBLESHOOTING.md`.

Doc lookup order is root → `.planning/` → `.planning/archive/` (first match
wins), so gate-consumed docs must live in one of those three roots. Active
planning docs go in `.planning/`; completed records go in
`.planning/archive/`.

---

## Repository layout

| Path | Purpose |
|---|---|
| `src/main.rs`, `src/cli.rs` | Binary entry, arg parsing, stderr-only tracing |
| `src/server.rs`, `src/server_runner.rs` | MCP surface: tool contracts, `INSTRUCTIONS`, dispatch, blocking pool, progress |
| `src/tools/` | Tool handlers: `report`, `query`, `structure`, `leads`, `finding`, `finding_read`, `project_control`, `health` |
| `src/sync/` | Indexing: manager, orchestration, health, reconcile, publish/CAS, revision, sweep, discovery, ignore rules |
| `src/store/` | The only SQLite boundary: connection guards, migrations, `schema.sql` (`CURRENT_SCHEMA_VERSION = 6`) |
| `src/watch/` | `notify` watcher, per-project `WatchState`, `WatcherHealth` |
| `src/evidence/` | Tree-sitter evidence extraction + normalization |
| `src/graph/` | Import resolution (Rust/Python/JS resolvers, `resolve_imports`) |
| `src/parse/`, `src/model/` | Language detection + `PACK_VERSION`; limits, profiles, spans, source models |
| `src/project/` | Project root resolution, activation marker, database path, `config.json` |
| `src/ignore_rules.rs` | Single source of truth for exclusions (discovery, watcher, sweep, reconcile) |
| `src/resources.rs` | `slugaudit://` resources, compiled in via `include_str!` from `examples/queries/` and `src/store/schema.sql` |
| `src/bin/` | Development gate binaries |
| `tests/` | Integration crates: `stdio_protocol`, `menu_tests`, `connect_tests` |
| `benches/` | Criterion targets: discovery, parsing, search, startup, memory |
| `examples/queries/` | Query cookbook **and** the `include_str!` source of `slugaudit://` resources |
| `.planning/ARCHITECTURE.md` | Authoritative design + invariants (gate-checked structure) |
| `.planning/PLAN.md` | The single active plan |
| `.planning/archive/DECISIONS.md` | Append-only decision log — add entries, never rewrite history |
| `.planning/archive/perf_baseline.json` | Consumed by `check_performance` — never delete |
| `.planning/archive/mcp_surface.json` | The committed `tools/list` + `resources/list` surface, consumed by `check_mcp_surface` — re-record deliberately with `--record` |
| `docs/CLIENTS.md` | Every `slugaudit connect` target and its config path/scope |
| `docs/TROUBLESHOOTING.md` | Operator runbook: slow calls, full-publish loops, sync budget, watcher unavailable, rebuild semantics |
| `.planning/slugaudit/` | Runtime DB state, gitignored — never documentation, never committed |
| `Cargo.toml`, `rust-toolchain.toml`, `clippy.toml`, `deny.toml` | Bins, benches, lint policy, pins, supply-chain policy |

---

## Development commands

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

### Gates

CI (`.github/workflows/quality.yml`, Rust 1.97.1, Linux + macOS) runs, in
order:

```bash
cargo fmt --all -- --check
cargo run --quiet --bin check_source_limits --locked   # LoC ceilings + exception annotations
cargo run --quiet --bin check_no_duplicates --locked   # duplicate commit subjects / test fn names
cargo run --quiet --bin check_docs_drift --locked      # ARCHITECTURE module map, test count, canonical docs
cargo run --quiet --bin check_producer_coverage --locked  # every evidence kind is constructed somewhere
cargo run --quiet --bin check_mcp_surface --locked     # tools/list + resources/list match the committed snapshot
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --bins --tests --all-features --locked
cargo test --test stdio_protocol --locked              # re-run alone so a transport break is its own check
cargo build --release --locked
```

Then `cargo-deny` (`check advisories bans sources licenses`),
`rustsec/audit-check`, `check_coverage`, and — Linux only, fails closed —
`cargo mutants` over `src/sync/revision.rs`, `src/sync/publish.rs`,
`src/sync/hash.rs`, `src/tools/context.rs`.

`check_performance` is a local gate, **not** wired into CI. Run it when
touching the discovery/parsing/search paths:

```bash
cargo run --bin check_performance --locked                 # criterion medians vs baseline, default 20%
cargo run --bin check_performance --locked -- --record     # deliberately re-baseline
```

Other local tools: `bash install.sh` installs to `${HOME}/.slugthug/slugaudit`;
`slugaudit update` downloads a release and verifies it against `SHA256SUMS`
before replacing the binary (`src/update.rs`).

---

## Testing and QA

### Layout

- Suites live beside their subject. Large ones are sibling files pulled in at
  the bottom of the production file:
  ```rust
  #[cfg(test)]
  #[path = "leads_tests.rs"]
  mod tests;
  ```
  Small modules keep an inline `#[cfg(test)] mod tests { use super::*; .. }`.
  Extra siblings append the concern (`publish_race_tests.rs`,
  `context_transaction_tests.rs`).
- Shared fixtures: `src/sync/test_support.rs` (`create_project`,
  `setup_project`, `sync_project`, `stored_paths`, `write`, `write_file`),
  `src/tools/test_support.rs` (`activated_project`), `src/graph/test_support.rs`,
  and `src/bin/check_docs_drift/test_support.rs`.
- Integration tests are separate crates in `tests/`; benches are criterion
  binaries with `harness = false`.

### House style

- One `#[test]` per behavior, named for the observable contract in snake_case,
  often opening `a_`/`an_`/`the_`.
- Arrange / blank line / act / blank line / assert; every file starts with
  `use super::*;`.
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
- `NoopProgressSink` is the default sink for every tool/publish call
  (`src/progress.rs`).
- Env mutation: `temp_env::with_var*` **plus** `util::TEST_ENV_LOCK`; tests
  touching `SESSION_ID` take `util::SESSION_TEST_LOCK`; `ToolCounters` are
  process-wide and cannot be reset (assert deltas); `process_limits()` is a
  `OnceLock`, so env changes after first use are ignored.
- `src/sync/race_hook.rs` is the deterministic no-sleep race barrier —
  `cfg(test)` and private to `sync`; `tools` tests roll their own `mpsc`
  barrier.
- Log capture goes through `store::test_capture` (`capture_at_level` /
  `capture_warns`). It is thread-local, but its isolation also rests on
  `PERMISSIVE_BACKSTOP`, a permanently registered permissive dispatcher: tracing's
  callsite-interest cache and max level are process-global, so without a second
  registered dispatcher a capture silently loses its own lines. Never delete the
  backstop as redundant. A test that **counts** captured lines must name its tool
  with a literal unique to it and filter the capture to that literal; a test whose
  capture assertion is absence-only must first emit and detect a control line, or it
  passes vacuously when the capture goes blind.
- Client-facing tests are **hermetic**. `tests/connect_tests.rs` runs the real
  binary against a temporary `HOME`/`SLUGTHUG_HOME`/`SLUGTHUG_CONFIG_HOME` with
  `PATH` *replaced* by a fake CLI per agent that records its argv, so no test can
  resolve or execute an installed client or read a real client config. Replacing
  `PATH` is the rule, not prepending to it — `src/connect_tests.rs`'s
  `with_fake_clis_on_path` / `with_no_cli_on_path` exist for that, and
  `tests/menu_tests.rs` is sandboxed the same way, because menu options 2 and 3
  run the connect path. `tests/stdio_protocol.rs` spawns the built binary and uses
  10-second receive timeouts (load-sensitive).

### Expectations

- Coverage: `cargo run --quiet --bin check_coverage --locked`, default gate
  **83** percent line coverage via `cargo llvm-cov` (requires
  `cargo-llvm-cov`). Override the threshold by passing a number on the command
  line for a single run. The gate tracks measurement — it catches regressions,
  it is not an aspirational target — so document any raise in
  `.planning/archive/DECISIONS.md`.
- Performance: 20 percent median regression allowance against
  `.planning/archive/perf_baseline.json`; startup and memory benches are
  measured but **not** gated.
- Property tests: only `src/graph/resolver/proptest.rs`; its seed corpus in
  `proptest-regressions/` is committed — never delete it.

---

## Runtime and tooling preferences

- **Rust 1.97.1** pinned in `rust-toolchain.toml` (minimal profile + rustfmt +
  clippy); edition 2024. There is **no declared MSRV** — do not invent one.
- Dependencies: 16 runtime + 4 dev (`Cargo.toml`).
  `tree-sitter-language-pack = "=1.14.3"` is the only exact pin;
  `src/parse/PACK_VERSION` is a hand-duplicated string that
  `src/parse/mod.rs::pack_version_matches_the_cargo_toml_pin` asserts against
  the manifest pin — update both together.
- Supply chain (`deny.toml`): no advisories ignored, no wildcards, duplicate
  versions denied outside a reviewed skip list, sources restricted to crates.io,
  licenses allow-listed. A new dependency must clear `cargo deny`.
  One dependency was considered and rejected on those grounds, and should not be
  re-proposed without new evidence: a **YAML writer** for `goose`'s config.
  `serde_yaml` was archived in 2024, and its maintained continuation `yaml_serde`
  (official YAML org, released 0.10.7 in Aug 2026) still vendors the C-derived
  libyaml core in-tree with real `unsafe`; the pure-Rust options
  (`saphyr`/`serde-saphyr`, `yaml-rust2`) are young with small adoption; the
  commonly-recommended fork `serde_yaml_ng` still depends on the deprecated
  `unsafe-libyaml` and has had no release since Sep 2025; and a
  deserialize→serialize round trip would strip the user's comments and
  formatting anyway, breaking the config it was meant to extend. `connect goose`
  therefore prints a paste-ready block instead — see
  `ConfigShape::GooseExtensions` in `src/connect_agents.rs`.
- Toolchain and lockfile updates are **their own deliberate change**, never a
  side effect of unrelated work, and they clear the same full gate as any other
  change.
- A new dependency that introduces transitive unsafe must be named in the
  release's third-party attribution list. `LICENSE`'s Notices clause
  (`LICENSE:71-79`) requires whoever receives a copy to get the licensor's
  `Required Notice:` lines as well, so the release ships those notices; the list
  is part of the release backlog in `.planning/PLAN.md`
  (“Production-readiness backlog” → “Release, versioning, supply chain”).
- Logs go to **stderr** only; `SLUGAUDIT_LOG_FORMAT=json` switches the
  subscriber to JSON (`src/main.rs`).
- Limits resolve in two layers: `SLUGAUDIT_PROFILE=adaptive|audit|lean` plus
  `SLUGAUDIT_<FIELD>` overrides, cached in a `OnceLock`
  (`src/model/limits.rs`), then `.planning/slugaudit/config.json` via
  `ProjectConfig::apply_to_limits` — where a project-file `profile` **replaces**
  the whole env-derived struct (`src/project/config.rs`).
- Releases are Linux x86_64 only (`.github/workflows/release.yml`), shipped with
  `SHA256SUMS`; `install.sh`/`slugaudit update` verify that checksum before
  installing.
- Prefer SlugAudit's **own MCP tools** when inspecting this repo: `report` for
  shape, `query` (read-only SQL over `files`/`evidence`/`dependency_edges`)
  instead of disk reads, `structure` for Tree-sitter pattern matches, and
  `leads` for mechanical counts. Join `evidence` to `files` on `file_id`.
