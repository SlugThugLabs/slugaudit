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
- Crate `slugaudit-mcp-rust`, version `1.1.0`, edition 2024, `publish = false`,
  `license-file = LICENSE` (non-SPDX PolyForm Noncommercial 1.0.0 plus the
  tool-use additional permission). `Cargo.toml`.
- Runtime data lives in `<project>/.planning/slugaudit/project.db` (+ sidecars):
  disposable derived data, excluded from discovery, gitignored via
  `/.planning/slugaudit/` (`.gitignore`). Findings are session-scoped and prior
  sessions' rows are marked `stale` (never deleted) on the first sync of each
  process/session, once per project database
  (`src/sync/manager_meta.rs::mark_prior_session_findings_stale`, gated by the
  manager's `prior_sessions_staled` state); later tool calls never repeat it.
  `finding_read` and `report` count only the current session; `finding_sessions`
  lists and adopts earlier sessions.

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
map, data flow, process lifecycle, concurrency, security model, and the full
numbered list of invariants. Read it before any structural change. This file
does not summarise it; duplicated summaries drift.

---

## Code conventions

### The RIGHT standard (replaces naive KISS)

Naive "Keep It Simple" often causes agents to strip out defensive error handling, validation, bounds checks, or telemetry to make code appear simpler. In SlugAudit, simplicity is never an excuse to discard rigor. We enforce **RIGHT**:

| Principle | Means | Guardrail |
|---|---|---|
| **Required** | Build for real, current needs | No features or structure for imagined futures |
| **Intact** | Don't rewrite working code without a real need | A real need is a bug, an owner requirement, or a rule it breaks. Removing unneeded code isn't a rewrite. |
| **Genuine** | Real functionality in product code | No demos, mocks or stubs in product code; test doubles stay in tests |
| **Hardened** | Keep the rigour the problem needs | Never strip error handling, validation, logging or maintainability to look simpler |
| **Trim** | Only necessary complexity | Cut extra abstraction, generality and tooling, never needed capability |

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
   `mechanic_counts` regardless of paging, rows cut by the response byte
   budget rather than any row ceiling (`query`'s 500-row cap was
   removed 2026-10-18; a page is cut only by `max_query_response_bytes`,
   and the agent writes its own `LIMIT`), `detail` clipped on a char
   boundary and flagged with `detail_clipped`, `next_offset` only when more
   remain, and a completion `tracing::info!`.
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
  module-map block in `.planning/MODULE_MAP.md` (every listed path must
  exist).
- Adding/removing a `src/**/*_tests.rs` file requires updating the
  “N `*_tests.rs` files share this pattern” sentence in `.planning/ARCHITECTURE.md`
  (the gate compares the claimed number against the real count).
- The canonical doc set exists: `README.md`, `AGENTS.md`,
  `.planning/ARCHITECTURE.md`, `.planning/MODULE_MAP.md`, `.planning/PLAN.md`,
  `.planning/archive/DECISIONS.md`, `docs/CLIENTS.md`,
  `docs/TROUBLESHOOTING.md`.

Doc lookup order is root → `.planning/` → `.planning/archive/` (first match
wins), so gate-consumed docs must live in one of those three roots. Active
planning docs go in `.planning/`; completed records go in
`.planning/archive/`.

---

## Repository layout

See `.planning/ARCHITECTURE.md` (*Subsystems and module layout*) and the
exhaustive manifest in `.planning/MODULE_MAP.md`.

---

## Development commands and gates

Build, test, lint, gate, coverage, and performance commands live in
`CONTRIBUTING.md` (*Build and test*, *Gates*). Keep the lists there, not here —
duplication drifts.

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

- Fuzzing: `fuzz/` is a **separate cargo workspace** (its own manifest, lockfile and
  `[workspace]` table), so nothing in it builds during `cargo build`, `cargo clippy` or
  `cargo test` at the root — it needs nightly and a sanitizer. Three targets cover the
  untrusted-input surfaces: `structure_query` (Tree-sitter query strings), `query_sql`
  (agent SQL, including authorizer-bypass shapes) and `request_arguments` (the
  `params.arguments` decode into the nine request types — this repository has **no frame
  parser to fuzz**: `src/main.rs` hands stdio to `rmcp`, which owns framing). Run one with

  ```sh
  cd fuzz && cargo +nightly fuzz run structure_query -- -max_total_time=60
  ```

  `fuzz/corpus/` is committed — curated seeds plus inputs fuzzing has discovered — so a
  scheduled run starts warm; `fuzz/target`, `fuzz/artifacts` and `fuzz/coverage` are
  gitignored. `.github/workflows/fuzz.yml` runs them on a cron and on dispatch, never per
  pull request. `fuzz/lsan-suppressions.txt` suppresses exactly one **dependency** leak;
  read it before deleting it — it carries the minimal reproducer, why the frames prove the
  allocation is not this crate's, and why the suppression is silently ineffective without
  `llvm-symbolizer`.

- Coverage: `cargo run --quiet --bin check_coverage --locked`, default gate
  **83** percent line coverage via `cargo llvm-cov` (requires
  `cargo-llvm-cov`). The measurement covers the shipped runtime; the dev-only
  `src/bin/check_*` binaries are excluded by `--ignore-filename-regex`, per the
  "Shipped artifact vs development-only" rule (see DECISIONS 2026-10-16).
  Override the threshold by passing a number on the command
  line for a single run. The gate tracks measurement — it catches regressions,
  it is not an aspirational target — so document any raise in
  `.planning/archive/DECISIONS.md`.
- Performance: 20 percent median regression allowance against
  `.planning/archive/perf_baseline.json`; startup and memory benches are
  measured but **not** gated.
- Property tests: only `src/graph/resolver/proptest.rs`; its seed corpus in
  `proptest-regressions/` is committed — never delete it.
- Large-repo soak: `tests/large_repo_soak.rs` builds a synthetic 10k-file
  project, syncs it through the real path (`SourceSyncManager::ensure_current`),
  and asserts the import completed inside `max_sync_wall_clock` with the whole
  file set indexed (row count, file kinds, parse outcomes, and the exact path
  set). The 10k case is `#[ignore]`d and **not scheduled in CI**:
  `cargo test --test large_repo_soak -- --ignored --nocapture` (~22 s of sync
  for the 10k files on the 12-core Ryzen 9 5900X it was measured on, in the
  unoptimized `test` profile; the budget is 1200 s). Run it when touching
  discovery, hashing, parsing, or publish. Its non-`#[ignore]`d companion
  (64 files, ~0.25 s) runs in the default suite and carries the same
  assertions, so the checks themselves are not soak-only.

### Mutation testing

CI runs `cargo-mutants` (Linux only, fails closed) over exactly six files:
`src/sync/revision.rs`, `src/sync/revision_support.rs`, `src/sync/publish.rs`,
`src/sync/hash.rs`, `src/sync/hash_io.rs`, `src/tools/context.rs`. That narrow
scope is a decision, not an unfinished widening — the plan's *Untrusted-input
rigor* item asks for a wider scope **or** a written reason for this one, and
this is the reason. Those six are the publish/revision/context correctness
core: revision identity, stale-finding invalidation, atomic publication, hash
types and in-memory digests, bounded `hash_file` / `stat_identity` reads, and
the freshness-verification entry point every state-bearing tool handler
shares. A mutant that survives there means a revision was published wrong or a
stale one was served, which is a class of defect no other file can produce.

`src/sync/hash_io.rs` and `src/sync/revision_support.rs` are not a widening:
the wave-3 R4 split (2026-10-19) moved `hash_file` and `stat_identity` out of
`hash.rs` into `hash_io.rs`, and the wave-4 size fix (2026-10-19) moved
`invalidate_stale_findings` out of `revision.rs` into `revision_support.rs`,
both without changing them. The six-file scope mutates the same
correctness-core functions the four-file scope did; `revision.rs`,
`publish.rs`, `hash.rs`, and `context.rs` keep their original responsibility.

Widening the scope multiplies CI minutes rather than adding a fixed cost:
`cargo-mutants` rebuilds the crate and re-runs the whole test suite once per
mutant, so the step costs mutant count × suite time. Measured with
`cargo mutants --list` (2026-09-30), the original four files hold 23 of the
crate's 2323 mutants (~1 percent); all of `src/sync/**` is 310 and all of
`src/tools/**` is 484, so widening to either is a 13x / 21x multiplier. The
mutants a wider scope would add sit in code the existing suites already drive
end to end (`discovery_tests`, `sample_tests`, `sweep_tests`,
`publish_*_tests`, the `tools/*_tests` suites), so that multiplier re-verifies
behavior the tests already pin, while the six files above are where a mutation
can slip past a green suite. The scope is defined in the step's comment in
`.github/workflows/quality.yml`, which carries the same decision — a file that
moves into the correctness core is added in both places, with the expectation
that the step's runtime grows proportionally.

Locally, one file at a time is the cheap way to iterate:

```bash
cargo install cargo-mutants
cargo mutants --file src/sync/hash.rs
```

---

## Runtime and tooling preferences

- **Rust 1.97.1** pinned in `rust-toolchain.toml` (minimal profile + rustfmt +
  clippy); edition 2024. There is **no declared MSRV** — do not invent one.
- Dependencies: 16 runtime + 4 dev (`Cargo.toml`).
  `tree-sitter-language-pack = "=1.18.0"` is the only exact pin (number verified
  against `Cargo.toml` on 2026-10-16; this sentence has drifted before — re-check
  it whenever the pack moves);
  `src/parse/mod.rs`'s `pub const PACK_VERSION` is a hand-duplicated string that
  the `check_version_consistency` gate asserts against the manifest pin —
  update both together.
- Supply chain (`deny.toml`): no advisories ignored, no wildcards, duplicate
  versions denied outside a reviewed skip list, sources restricted to crates.io,
  licenses allow-listed. A new dependency must clear `cargo deny`.
  Every third-party action in `.github/workflows/` is pinned to the full commit
  SHA behind its release tag with a `# vX.Y.Z` comment (`.github/dependabot.yml`
  proposes the next pin, weekly, for both `cargo` and `github-actions`); the one
  exception is `dtolnay/rust-toolchain`, which publishes no release tags and is
  therefore re-pinned by hand. The exact-pinned parser pack has no other automated
  bump path — `cargo update` cannot move an `=` requirement — so its only
  automatic bump arrives as a Dependabot PR, and that PR must move
  `src/parse/mod.rs`'s `PACK_VERSION` with it.
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
  `Required Notice:` lines as well, so the release ships those notices —
  generated by the release workflow from the dependency graph
  (`docs/about.toml` config, whose accepted-licence list mirrors `deny.toml`;
  template `docs/third-party-notices.hbs`) and attached to the release.
- Logs go to **stderr** only; `SLUGAUDIT_LOG_FORMAT=json` switches the
  subscriber to JSON (`src/main.rs`).
- Limits resolve in two layers: `SLUGAUDIT_PROFILE=adaptive|audit|lean` plus
  `SLUGAUDIT_<FIELD>` overrides, cached in a `OnceLock`
  (`src/model/limits.rs`), then `.planning/slugaudit/config.json` via
  `ProjectConfig::apply_to_limits` — where a project-file `profile` **replaces**
  the whole env-derived struct (`src/project/config.rs`).
- Releases are Linux x86_64 only (`.github/workflows/release.yml`), shipped with
  `SHA256SUMS` (the binary only — its format is load-bearing for
  `install.sh`/`slugaudit update`, which verify it before installing), `LICENSE`,
  a CycloneDX SBOM, third-party notices and a build attestation. Those are two
  different claims: the checksum is integrity, the attestation is authenticity.
  `docs/RELEASING.md` documents how a consumer verifies each, and what a green
  reproducibility job does and does not mean.
- Prefer SlugAudit's **own MCP tools** when inspecting this repo: `report` for
  shape, `query` (read-only SQL over `files`/`evidence`/`dependency_edges`)
  instead of disk reads, `structure` for Tree-sitter pattern matches, and
  `leads` for mechanical counts. Join `evidence` to `files` on `file_id`.
