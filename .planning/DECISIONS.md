# Decision log

The dated decision log required by plan-audit amendment 21.1 ("a dated plan
decision log"). Every deliberate choice that deviates from, pins, or adds
specificity to `IMPLEMENTATION_PLAN.md` gets an entry here. Entries are
append-only; superseded decisions are marked and left in place.

Format: date — title; status; context; decision; rationale; consequences.

## 2026-08-02 — Coverage gate threshold set at 89%

- **Status**: decided
- **Context**: plan-audit required a coverage policy; the plan itself did
  not fix a number.
- **Decision**: CI gates line coverage at ≥ 89%
  (`cargo llvm-cov --fail-under-lines 89`). Measured real coverage on
  2026-08-02 was 91.88%; the gate is measured-minus-margin, not an
  aspirational target.
- **Rationale**: a margin below real coverage avoids flaky CI while still
  catching large regressions.
- **Consequences**: the number is recalibrated deliberately when real
  coverage moves; it must not be ratcheted down to silence a regression.

## 2026-08-03 — Rust import resolution made workspace-aware

- **Status**: decided
- **Context**: `crate::`/`super::`/`self::` resolution was a directory
  heuristic resolving ~3% of a real workspace's imports.
- **Decision**: anchor `crate::` at the owning crate's `src` by walking up
  to the nearest indexed `Cargo.toml`; support glob imports, non-`mod.rs`
  module trees, trailing-item-name prefix fallback, and multi-crate
  workspaces. Verdicts for `super::`/`self::` remain `"Low"` confidence
  because `mod` declarations are not read.
- **Rationale**: real resolution on a real workspace (3% → 97%) without
  pretending to be a compiler.
- **Consequences**: `.planning/README.md` "Dependency-edge resolution scope"
  now describes workspace-aware behavior.

## 2026-08-03 — Oversized modules split to restore the source-size gate

- **Status**: decided
- **Context**: several production files crossed the 200-code-line limit
  during development; the gate had been allowed to rot.
- **Decision**: split modules by ownership (sync/publish into
  publish/revision/hash/manifest; store into connection/migrations/
  repositories) and restored `tools/check_source_limits.sh` as a hard CI
  step. Splits must follow `IMPLEMENTATION_PLAN.md` §2.2 (one primary
  reason to change) — no hiding logic in macros or giant constants.
- **Rationale**: the limit exists to keep ownership boundaries legible; an
  exception requires a documented `slugaudit-line-exception` reason.

## 2026-08-04 — Audit lifecycle hardened; failures surfaced

- **Status**: decided
- **Context**: plan-audit findings around audit lifecycle and project
  indexing had been accepted but not yet addressed.
- **Decision**: implement the audit corrections (project indexing and
  lifecycle hardening) before further feature work, per §19 phase rules
  (no next phase while an unresolved correctness failure exists).

## 2026-08-09 — Incremental sync optimized; clippy warnings cleared

- **Status**: decided
- **Context**: every tool call re-discovered and re-hashed every file.
- **Decision**: add incremental reconciliation — unchanged files keep their
  hashes and derived evidence; only dirty/deleted paths are re-processed.
  Gate must remain `clippy -D warnings` clean.

## 2026-08-10 — Watcher-backed barrier sync becomes the production path

- **Status**: decided
- **Context**: incremental reconcile raced with events arriving during
  reconciliation (events lost or desynced watcher state).
- **Decision**: `SourceSyncManager` owns a per-project filesystem watcher.
  When trusted and events are pending, `reconcile_dirty_paths` runs; a full
  publish runs only when the watcher is untrusted or unavailable. A barrier
  loop (`sync_with_barrier`, capped at `MAX_BARRIER_LOOPS = 16`) drains
  events arriving during reconciliation and marks the watcher `Desynced`
  under a pathological event producer. Watcher health is exposed via the
  `health` tool.
- **Rationale**: correctness under concurrent edits without losing events;
  a bounded loop prevents an unbounded drain.

## 2026-08-10 — Resolver split into a language-agnostic trait architecture

- **Status**: decided
- **Context**: `src/graph/resolver.rs` (522 LoC) violated the source-size
  gate and conflated languages.
- **Decision**: split into `registry.rs` + `generic.rs` + `python.rs` +
  `js.rs` + `path_helpers.rs`, with a per-language trait (the
  language-agnostic resolver architecture) so new languages register
  without touching existing resolvers.
- **Consequences**: `ARCHITECTURE.md` documents the five-file resolver
  structure.

## 2026-08-10 — Runtime databases removed from git tracking

- **Status**: decided
- **Context**: the per-project runtime database
  (`.planning/slugaudit/project.db*`) was accidentally committed.
- **Decision**: the runtime database is never versioned — it is gitignored,
  machine-local, and excluded from discovery by `src/sync/discovery.rs`
  itself.
- **Rationale**: the index is reproducible from source; per §21.4
  corruption policy the database is derived data, not an artifact.

## 2026-08-10 — Windows owner-only permissions deferred

- **Status**: decided (supersedes nothing; parked)
- **Context**: Windows `CreateFileW` + `SECURITY_ATTRIBUTES` owner-only
  permission work was proposed for `src/store/connection.rs`.
- **Decision**: defer Windows-specific permission hardening; keep the Unix
  owner-only `0o600` path as the reviewed behavior. Revisit only if a
  Windows consumer materializes.
- **Rationale**: the product targets stdio MCP hosts on developer
  workstations; no Windows requirement exists yet.

## 2026-08-10 — Project control moved into the MCP surface; CLI simplified

- **Status**: decided
- **Context**: an earlier iteration exposed `enable`/`disable` as human
  CLI subcommands.
- **Decision**: enable/disable is now the `project_control` MCP tool
  (`action = "on"` creates the marker and runs the first import
  immediately; `action = "off"` removes the marker and purges the
  database under an exclusive lock). The CLI is `serve` (default),
  `connect [AGENT]` (register as the MCP server for Claude Code/Grok/
  Codex), `install`, and `help`. No human-facing sync/rebuild command
  exists.
- **Rationale**: §1 requires exactly one human-facing control; everything
  else must flow through AI-invoked MCP calls.

## 2026-08-10 — Performance baseline added (Task 9.2)

- **Status**: decided
- **Context**: Task 9.2 and plan-audit PHASE-09 required reproducible
  benchmarks with recorded budgets and a regression policy; neither
  existed.
- **Decision**: add `criterion` as a dev-dependency with four
  `harness = false` bench targets (`discovery`, `parsing`, `search`,
  `sync`) sharing a deterministic fixture generator in
  `benches/common/mod.rs`. Record the machine, fixture definition, exact
  command, results, budgets, and regression thresholds in
  `.planning/PERFORMANCE.md`. Benchmark builds stay separate from
  correctness gates (`cargo test --all-targets` compiles, never runs,
  them).
- **Rationale**: deterministic splitmix64-seeded fixtures make runs
  reproducible across machines; explicit budgets and a ≥ 20% median
  regression threshold give the numbers a gate they can actually fail.
- **Consequences**: the sync benches run with `SourceSyncManager::new()`
  (no watcher), so unchanged/changed sync numbers are worst-case
  full-verification publishes; the watcher-trusted incremental path
  remains covered by functional tests.

## 2026-08-10 — Freshness tests made deterministic (watcher-timing flake)

- **Status**: decided
- **Context**: `tools::context::tests::{verified_read_write_has_the_same_protection,
  a_stale_synced_handle_fails_loudly_instead_of_returning_mismatched_data}`
  failed intermittently under parallel full-suite runs while always passing
  in isolation. Both used the global watcher-backed manager for the second
  sync: if the async `notify` event for the modified file had not reached
  the watcher thread before the second `ensure_current`, the revision did
  not move and the stale handle was wrongly accepted.
- **Decision**: those tests sync through a local
  `SourceSyncManager::new()` (no watcher) via a `synced_locally` helper, so
  every `ensure_current` is a full publish and a content change always
  moves the revision. Found while validating the Task 9.2 benchmark work;
  no production code changed.
- **Rationale**: the tests assert stale-handle rejection, not watcher event
  delivery; the concurrent-publish isolation property remains covered by
  `context_race_tests.rs`.
- **Consequences**: full suite passes consistently (verified twice under
  parallel load).

## 2026-08-10 — Phase 12 acceptance fixture and golden manifest (Task 12.1)

- **Status**: decided
- **Context**: plan-audit PHASE-12 required a versioned fixture contract
  with a golden manifest, evidence counts/statuses, partial-language
  expectations, and zero-skipped critical tests; nothing existed.
- **Decision**: check in a 28-file polyglot fixture at
  `tests/fixtures/multilang/` (rust, python, typescript, go, ruby — all in
  the old Python eight-language set — plus **javascript**, the one
  language outside the old eight; malformed Python source; a Python and a
  JavaScript circular import pair; config, docs, scripts, and a binary
  file). Add `tests/fixture_contract.rs`, which publishes a temp-dir copy
  of the fixture and asserts the database matches the versioned golden
  manifest (`MANIFEST.json`, contract version 1), with a documented
  `SLUGAUDIT_REGEN_MANIFEST=1` regeneration mode that dumps raw
  dependency edges for hand review.
- **Rationale**: golden-manifest discipline (pin pack output, review
  regenerations) is the only way exact evidence expectations stay honest
  across a 306-parser pack. The fixture deliberately includes languages
  SlugAudit does not resolve (Go/Ruby) so partial capability is asserted
  as `unsupported_language_unresolved_count`, not hidden.
- **Consequences**: the contract test caught a real test bug on first run
  (database placed inside the project root got indexed as binary; moved to
  `.planning/slugaudit/` matching production). The manifest is
  deterministic (5 consecutive identical passes).

## Open items tracked in this log

- **Performance baseline (Task 9.2)** — **done 2026-08-10**: `benches/`
  (discovery, parsing, search, sync) + `.planning/PERFORMANCE.md` exist;
  budgets proposed, startup and peak-memory budgets still unmeasured
  (noted in PERFORMANCE.md).
- **Phase 12 acceptance fixture (Task 12.1)** — **done 2026-08-10**:
  fixture + golden manifest + contract test as above. Task 12.2 (the
  real-MCP workflow test: activate → initialize → report → query →
  structure → finding → modify → stale-finding verification) and Task
  12.3 (the complete release gate) remain.
- **Full-crate mutation baseline** — CI mutation is scoped to
  revision/publish/hash/context and `continue-on-error`; a full-crate
  baseline and survivor triage is an ongoing workstream.
- **`criterion` addition** — when benchmarks are added, criterion becomes a
  dev-dependency; record its license (`Apache-2.0`/`MIT`) against the
  allow-list at that time.

## 2026-08-10 — notify 7 → 8.2.0 to eliminate RUSTSEC-2024-0384

- **Status**: decided
- **Context**: `cargo deny check advisories` failed on RUSTSEC-2024-0384
  (`instant` unmaintained), reached only through `notify 7` → `notify-types
  1.0.1` → `instant 0.1.13`. `cargo update` cannot fix it; `instant` has no
  safe upgrade and deny.toml forbids ignores (`ignore = []`).
- **Decision**: bump `notify` to `8` (lock: 8.2.0), which drops the
  `notify-types`/`instant` path. Same-day compatible lock refresh
  (`cargo update`) collapsed thiserror 1.x, windows-sys 0.45, jni-sys 0.3.1,
  and cesu8 duplicates.
- **Rationale**: the repo policy bans advisory ignores; the watcher API
  surface used (`RecommendedWatcher::new`, `watch`, `Config`, `EventKind`)
  is unchanged in notify 8, so `src/watch/manager.rs` required no changes.
- **Consequences**: advisory gate green; duplicate outcome recorded per
  policy 5 — notify 8 adds windows-sys 0.60/targets 0.53 entries alongside
  ring's 0.52.6 (warn-level, platform-only, Windows targets never built
  here). Full gate validated on pinned 1.97.1: build, 281 tests passed,
  `cargo deny check` all-ok, clippy -D warnings clean.

## 2026-08-10 — Incremental reconcile re-detects binary files; coverage gate restored

- **Status**: decided
- **Context**: a due-diligence audit (2026-08-10) found two gate/quality
  problems. (1) `reconcile_dirty_paths` hardcoded `FileKind::Indexed` for
  every dirty path, so a binary file modified on disk was re-indexed as
  lossy UTF-8 text — the initial import's `discover()` sniffs binary-ness,
  but the incremental watcher path did not. (2) Real measured line
  coverage had dropped to 82.77% against the 89% gate because the newest
  safety-critical modules (`project_control` 27.87%, `health` 45.83%,
  `sync/manager` 60.62%) shipped with thin or no tests.
- **Decision**: (1) `reconcile_dirty_paths` now calls
  `discovery::sniff_kind` (the same NUL-byte detector the initial import
  uses) for every dirty path, with a regression test
  (`modified_binary_dirty_path_stays_binary_excluded`); `ReconcileError`
  gains a `Discovery` variant for the read failure. (2) Added tool-level
  tests across the low-coverage surface: `project_control` enable/disable
  end-to-end, `health`'s database-backed half, `SourceSyncManager`
  observability and publish-failure paths (via the `race_hook` seam),
  `with_verified_*` missing/corrupt-database failure paths,
  `WatchManager::handle_event` via constructed `notify` events,
  `install`/`connect` (env-scoped with the new `temp-env` dev-dependency
  and a shared test env lock), and `structure`'s empty-query / truncation /
  no-content branches.
- **Rationale**: binary classification must have one source of truth
  shared by full and incremental sync; the coverage gate's own rationale
  ("gate is measured-minus-margin, not aspirational") demands new code be
  tested before it lands.
- **Consequences**: measured line coverage is 89.12% (gate green again —
  the `--fail-under-lines 89` check compares line, not region, coverage);
  suite grew 279 → 329 tests.  `temp-env` is a dev-dependency only
  (Apache-2.0/MIT, recorded in `DEPENDENCIES.md`). `connect`'s agent-CLI
  invocation paths are tested with inert fake `claude`/`grok`/`codex`
  scripts prepended to `PATH` under the env lock, so a real agent
  registration is never touched; the missing-CLI error carries a guard
  that skips on machines where an agent CLI is actually installed.

