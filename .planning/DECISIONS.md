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

- **Status**: decided (superseded 2026-08-10 — the fixture and contract
  were removed; see the "Phase 12 fixture removed" entry below)
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
  deterministic (5 consecutive identical passes). *Superseded: the
  self-referential golden-manifest model (tool-generated ground truth,
  human hand-review, per-language fixture) was removed on the same date —
  see the dedicated entry below.*

## 2026-08-10 — Phase 12 fixture removed; tree-sitter pack bumped to 1.14.3

- **Status**: decided
- **Context**: the Phase 12 acceptance fixture
  (`tests/fixtures/multilang/` + `tests/fixture_contract.rs`) was
  reviewed against the product's actual thesis (an evidence server for
  the AI) and found to be testing the wrong architecture. (1) The golden
  manifest was **generated by the tool itself** via
  `SLUGAUDIT_REGEN_MANIFEST=1` and then hand-reviewed by a human — it
  certifies self-consistency ("the tool's output equals the tool's
  output"), not fidelity to reality; a systematic blind spot would be
  baked into the manifest and the contract would pass forever with the
  bug installed. (2) It pinned a hand-picked per-language list
  (rust/python/typescript/go/ruby/javascript — the "old Python
  eight-language set") when the tool is **language-agnostic by design**:
  `src/evidence/normalize.rs::extract` ties into the pack's generic
  `process()` (one function, all 306/371 languages, zero per-language
  code), and the only per-language surface is the import resolver
  (python/js/rust + generic fallback), which exists because the pack
  itself only extracts imports for ~6 languages. (3) Every pack bump
  forced the human regenerate-and-hand-review ceremony the product was
  built to eliminate.
- **Decision**: delete `tests/fixture_contract.rs` and
  `tests/fixtures/multilang/` (29 files + the contract test); clean the
  references in `.planning/README.md`, `.planning/RELEASE_CHECKLIST.md`,
  and this log; and bump `tree-sitter-language-pack` from `=1.13.7` to
  `=1.14.3` (306 → 371 languages; same tree-sitter 0.26 pin, same
  `process`/`ProcessConfig`/`get_parser` API surface, same default
  features `dynamic-loading` + `download`, verified against the crate
  source). `PACK_VERSION` in `src/parse/mod.rs` is updated in lockstep
  (its guard test asserts both).
- **Rationale**: the tool's real fidelity test is an LLM hunting real
  bugs with the tool and finding them — not a manifest the tool wrote
  about a fake repo. The language-agnostic machinery is already covered
  by the general suite; the fixture tested a per-language model that no
  longer exists. Removing it unblocks pack upgrades with zero code
  changes and no human ceremony.
- **Consequences**: test count drops by the one integration test
  (`fixture_contract`); coverage is re-measured after the change and the
  gate stays green (the fixture's coverage contribution is confined to
  the same publish/analyze paths already exercised by the unit suite).
  The perf regression gate was re-run against the 1.14.3 engine and
  PASSES at worst 1.02x against the 1.13.7-era baseline (several benches
  measured faster), so the committed `perf_baseline.json` remains a
  valid bound; the baseline itself is not re-recorded in this change —
  see the note added to `.planning/PERFORMANCE.md` and re-record
  deliberately before any future release if the engine continues to
  move. The per-revision `parser_pack_version` recording — the
  mechanism that forces existing projects to re-analyze exactly once
  after a pack bump — is untouched and now fires for the 1.13.7 → 1.14.3
  upgrade.

## Open items tracked in this log

- **Performance baseline (Task 9.2)** — **done 2026-08-10**: `benches/`
  (discovery, parsing, search, sync) + `.planning/PERFORMANCE.md` exist;
  budgets proposed, startup and peak-memory budgets still unmeasured
  (noted in PERFORMANCE.md).
- **Phase 12 acceptance fixture (Task 12.1)** — **removed 2026-08-10**:
  the fixture, golden manifest, and contract test were deleted (see the
  "Phase 12 fixture removed" entry above); the real-MCP workflow test
  (Task 12.2) covers the acceptance sequence and remains. Task 12.3 (the
  complete release gate) was executed 2026-08-10 and remains recorded in
  `.planning/RELEASE_CHECKLIST.md`.
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

## 2026-08-10 — health tool made genuinely read-only; Task 12.2 completed; release gate run

- **Status**: decided
- **Context**: the audit flagged that `health`'s docs claimed it "never
  cause[s] a sync, never emit[s] MCP progress", but supplying a `path` ran
  `ensure_current` — which can publish a new revision on a modified
  project. A health check that perturbs the state it reports on is
  misleading. Separately, the real-MCP acceptance sequence (Task 12.2)
  stopped short of the final act (modify → finding stale), and the release
  gate (Task 12.3) had never been executed.
- **Decision**: (1) `health` with a path is now strictly read-only:
  `compute_project_state` resolves the project root, reads watcher state
  via `watch_state_for` (never registering a watch or changing health),
  and reads database fields read-only, degrading to None/-1 when the
  database doesn't exist; it never calls `ensure_current`. Unregistered
  projects report `NoActiveProject` while still surfacing DB fields.
  (2) `tests/stdio_protocol.rs` extended with the modify → sync → finding
  `stale` act and a restart test (fresh server serves the same revision).
  (3) The release gate was executed and recorded in
  `RELEASE_CHECKLIST.md` (2026-08-10): all gates green, coverage 89.32%,
  release sha256 `cee25220cf8888e06e4b41c9de2d9f42252fafe66455b14b4b1371203966aa39`.
- **Rationale**: observability must be side-effect-free; acceptance must
  prove the full lifecycle including invalidation, and a release gate is
  only meaningful if executed and recorded.
- **Consequences**: two new regression tests pin the read-only guarantee
  (`health_with_a_path_never_triggers_a_sync`, which would fail under the
  old behavior, and `an_enabled_but_never_synced_project_degrades_gracefully`);
  `health`'s `sink` parameter is now unused by the path branch (kept for
  the tool-call contract). `cargo-mutants`, `cargo-nextest`, and
  `cargo-geiger` are not installed locally — the checklist records the
  equivalents run and marks those three items with reasons; a dedicated
  mutation run is required before tagging a release.

## 2026-08-10 — Wall-clock timeouts on publish/reconcile (last Reliability item)

- **Status**: decided
- **Context**: the due-diligence audit's Reliability category was open on
  one item: a pathological repo (giant directory tree, pathological event
  producer, adversarial single file) could stall a tool call
  indefinitely because discovery, sampling, and reconciliation had no
  wall-clock budget — only the `query`/`structure` tools were deadline-
  bounded.
- **Decision**: follow the codebase's own `query`/`structure` pattern
  rather than introducing threads or async cancellation: add
  `ResourceLimits.max_sync_wall_clock` (default 60 s), a `util::Deadline`
  helper (`started` + `budget`, `exceeded() -> Option<Duration>` using
  `elapsed >= budget` so zero/nanosecond budgets trip deterministically),
  and cooperative deadline checks at every hot-loop boundary: the
  discovery walk (`discover_with_deadline`), per-file sampling
  (`sample_all_with_deadline`), per-file tree-sitter analysis in
  `build_upserts_and_deletions` (which now returns `Result` — parse
  failures stay evidence, only `TimeBudgetExceeded` is an error),
  `diff_against_stored` entry, per-dirty-path and per-barrier-iteration
  reconcile (`reconcile_dirty_paths_with_deadline` +
  `sync_with_barrier_with_deadline`), and the CAS retry loop. One
  deadline spans all CAS retries (`publish_with_limits`); one shared
  deadline spans all barrier iterations (manager::reconcile). Existing
  public signatures are unchanged — `publish`/`discover`/`sync_with_barrier`
  delegate with default limits; new `_with_deadline` variants are
  `pub(crate)`.
- **Rationale**: cooperative checks match the established pattern, keep
  the architecture single-threaded per tool call, and are deterministically
  testable with zero/nanosecond budgets (7 new tests in
  `src/sync/timeout_tests.rs`: publish, reconcile, barrier, and discovery
  all fail with `TimeBudgetExceeded` under `Duration::ZERO`). The
  malformed-source-is-evidence invariant is untouched — the only new
  publish failure mode is the budget itself.
- **Consequences**: budgets are per-phase, not per-call: the common paths
  are single-phase (Healthy→reconcile ≤ 60 s; Unavailable→publish ≤ 60 s);
  the rare untrusted path (publish + post-verification drain) can reach
  ~120 s. Nothing is unbounded anymore — the acceptance criterion is
  "a pathological repo cannot stall a tool call", not "always under one
  budget". A single pathological file's tree-sitter parse is not
  interruptible mid-parse (same accepted limitation as `query`/
  `structure`). `sample.rs` grew to 206 code lines and carries a
  `slugaudit-line-exception` (one cohesive sampling pipeline — splitting
  would scatter `to_file_record`'s evidence capping away from the loop
  that feeds it); `publish_race_tests.rs` stayed gate-clean (198 lines)
  by inlining its far-future deadline rather than adding an exception.
  Full gates green: 337 lib tests (7 new timeout + 2 deadline unit),
  fmt, clippy `-D warnings`, source-limits PASS, no-duplicates PASS.

## 2026-08-10 — Coverage gate made self-documenting; line coverage restored to 90.3%

- **Status**: decided
- **Context**: a due-diligence pass concluded the `--fail-under-lines`
  gate was "enforcement-disconnected" (printed 87.7% but exit code 0 at a
  89 threshold). Reading `cargo-llvm-cov` 0.8.7's source and the JSON
  report showed the flag compares the merged JSON `totals.lines.percent`
  (89.59% at the time), while the `--summary-only` text table prints
  region-weighted columns — the printed number was simply being misread
  as the gated one.
- **Decision**: (1) gate through `tools/check_coverage.sh`, which runs
  `cargo llvm-cov --all-targets --all-features --json`, extracts the
  merged line percent, prints it ("line coverage: 90.31% … gate 89.0%"),
  and fails below the threshold — the gated value is now unambiguous and
  independent of the flag's exact semantics; CI's coverage step calls the
  script. (2) Add tests for the uncovered timeout-path branches: corrupt
  database auto-recovery (`discard_corrupt_database` + republish),
  unopenable database error, Healthy+reconcile-failure → `Desynced`,
  reconcile CAS `StaleBaseline` fail-closed, and `sniff_kind` read-error
  handling (6 new tests; suite 337 → 343).
- **Rationale**: a gate whose measured number can't be misread is the fix;
  the tests cover the branches a real deployment can hit (corruption,
  unopenable db, reconcile failure) rather than defensive lines with no
  trigger.
- **Consequences**: line coverage 89.59% → 90.31% (gate green with
  margin). Remaining uncovered lines in the four sync files are defensive
  by inspection: analyze's load/parse-failure branches (no grammar in the
  pack fails to load — probed 30+ detected languages, all available),
  discovery's mid-walk filesystem-race skips, manager's post-drain failure
  mapping (no test seam after publish commits), reconcile's dead
  `exclude_count == 0` fallback behind the empty-set early return, and the
  unused public `activate` method.

## 2026-08-10 — Performance regression gate wired into CI (Task 9.2 completion)

- **Status**: decided
- **Context**: the Task 9.2 baseline recorded budgets and a ≥ 20% median
  regression threshold, but nothing enforced either — the numbers had no
  gate they could actually fail.
- **Decision**: add `tools/check_performance.sh`, which runs the four
  criterion benches at a reduced sample size (`--sample-size 10
  --warm-up-time 1 --measurement-time 3`), parses criterion's
  `new/estimates.json` medians, and fails when any bench exceeds the
  recorded median by more than the threshold (default 20%) or its
  recorded release budget. The baseline lives in a committed
  machine-readable file (`.planning/perf_baseline.json`, derived from
  PERFORMANCE.md's 2026-08-10 tables); `--record` regenerates it
  deliberately. CI runs the gate as a regular step.
- **Rationale**: same discipline as the coverage gate — the gated numbers
  are printed explicitly, and re-recording is a deliberate, documented
  act, never a silent way to hide a regression. The reduced-sample CI
  protocol is intentionally noisier than the recording protocol, which is
  exactly why the looser 20% threshold gates.
- **Consequences**: first gate run (2026-08-10, on the recording machine)
  PASS — worst ratio `sync_40/first_sync` 1.14x (noise), everything else
  ≤ 1.03x; the post-baseline wall-clock timeout instrumentation showed no
  measurable regression. Cross-machine caveat documented: a runner on
  different hardware must re-record the baseline or the gate will be red
  by construction.

## 2026-08-11 — Codebase audit corrections applied to the plan (§22)

- **Status**: decided
- **Context**: a full-source audit of the implemented codebase (not of the
  plan's prose) found 13 concrete defects and a stale plan header.
- **Decision**: append `IMPLEMENTATION_PLAN.md` §22 with the corrections
  and update the plan's status header. The blocking items are: C1 — the
  resolver's `contains("from")` substring gate drops Python imports whose
  module path contains `from` (correctness); C2 — up to three tree-sitter
  parses per file (performance); C3 — Task 9.1 parallel parsing was never
  built, and the 60 s sync deadline makes large-repo first imports the
  most likely first production failure; C6 — CI runs the criterion benches
  twice and the mutation gate can never fail.
- **Consequences**: §22.1–22.14 become acceptance items; C6(a)/C8/C10 are
  CI/tooling changes, C1/C2/C7/C9/C11 are code changes, C3/C12 are
  budget/benchmark changes. Full details, file locations, and fix
  prescriptions are in the plan section itself.

## 2026-08-10 — License decision: PolyForm Noncommercial 1.0.0 applied

- **Status**: decided
- **Context**: `Cargo.toml` declared `license =
  "PolyForm-Noncommercial-1.0.0"` but no `LICENSE` file existed — the
  metadata was not legally applied — and the decision was still listed as
  open in `.planning/README.md`.
- **Decision**: keep `PolyForm-Noncommercial-1.0.0`. Add the full official
  license text as `LICENSE` (with a `Required Notice` line — copyright
  SlugThugLabs), reference it from `README.md`, and close the open item.
  Noncommercial use is free; commercial use requires a separate license
  from the copyright holder (a commercial license channel is the intended
  revenue path).
- **Rationale**: the goal is shared source with a reserved commercial
  revenue path; permissive licensing would forfeit that. The license is
  not OSI-approved open source, so adoption is deliberately gated to
  noncommercial users and organizations.
- **Consequences**: `cargo deny check licenses` continues to pass
  (already allow-listed); `cargo package` ships the `LICENSE` file; any
  commercial distribution requires executing a commercial license with
  the copyright holder first.


## 2026-08-12 — Watcher scope and ignore rules aligned with discovery

- **Status**: decided
- **Context**: the audit flagged that the filesystem watcher watched every
  directory under a project root (including `target/`, `node_modules/`,
  `.git/`) while discovery only indexes the walker's file set, and that
  incremental reconcile indexed gitignored files a fresh publish skips
  (same tree, two different databases depending on which sync path ran).
  Chosen scope: "full fix, also prune the watch."
- **Decision**: one shared ignore source of truth
  (`src/ignore_rules.rs`): the hardcoded exclusions (SlugAudit's own
  data dir, VCS internals, scratch files) and a per-directory matcher
  that mirrors the `ignore` walker's semantics — nested `.gitignore`
  scoped to its own directory, `.ignore` overriding `.gitignore`
  regardless of depth, and ignored parents pruning their subtrees.
  `src/watch/scope.rs` enumerates the indexable directory set with the
  same walker discovery uses; `WatchManager` watches only that set
  (pruning on `.gitignore`/`.ignore` changes via `refresh_scope`, which
  must run on tool threads because `notify`'s `unwatch` blocks on the
  event loop), and drops events for ignored paths. Incremental reconcile
  skips ignored dirty paths (deletions still processed, so newly-ignored
  files converge out). A scope change marks the project
  `NeedsVerification` so the next sync does a full publish — files may
  have changed inside re-added directories while they were unwatched.
- **Rationale**: discovery, the watcher, and reconcile must agree on the
  indexable set by construction; a single walker config and shared
  exclusions are the only way to guarantee that.
- **Consequences**: watch-descriptor usage drops to the indexable
  directory set (this repo ~3,000 dirs to a few hundred); incremental
  and full-publish file sets can no longer drift; the known edge is that
  creating a `.gitignore` inside a currently-pruned directory won't fire
  an event, so the refresh waits for the next ignore-file change or full
  verification.

## 2026-08-12 — Dev tooling cut over to Rust bins; no shell scripts, no Python interpreter

- **Status**: decided
- **Context**: the dev pipeline ran four quality gates via
  `bash tools/check_*.sh` wrappers around embedded `python3 - <<'PY'`
  heredocs plus a `python3 smoke_test.py` reference in `PACKAGING.md`.
  This violates the project's "no helper logic in another language"
  rule on a toolchain level: a contributor with `cargo` but no `python3`
  cannot run `tools/check_no_duplicates.sh`, the coverage gate, the
  perf gate, or the source-limits gate locally; CI assumes a Python
  interpreter in the image that is documented nowhere; reviewers
  context-switch into Python to understand what a CI pass actually is.
- **Decision**: every gate that lived in `tools/*.sh` is now a first-class
  `cargo run --bin <name> --locked` bin at `src/bin/<name>.{rs,/}`.
  - `src/bin/check_source_limits/{main,counter,tests}.rs` — directory
    layout so the 280+ LoC state machine + counter + helpers + tests
    fit under the 200-LoC source-size cap without broad exclusions.
  - `src/bin/check_no_duplicates.rs` — single file (276 LoC, annotated
    `slugaudit-line-exception: approved-by=agent; reason=the bin owns
    two orthogonal gate inputs…`).
  - `src/bin/check_coverage.rs` — single file.
  - `src/bin/check_performance/{main,estimates,format,tests}.rs` —
    directory layout (250 LoC `main.rs` carries the exception; the
    comparator + threshold + record-mode argv parsing live in one place
    because the bin's user-visible surface is a single CLI invocation).
  - `tools/check_*.sh` are deleted (the directory is now empty).
  - `.github/workflows/quality.yml` invokes each gate as
    `cargo run --quiet --bin <name> --locked`; the printf-style stdout
    stays byte-compatible (CI name mentions and prior log scrapers
    keep working — verified by `diff` of full bin output against the
    replaced shell script).
  - `PACKAGING.md` drops the `python3 smoke_test.py` reference in favor
    of `cargo test --test stdio_protocol` (the real Rust
    subprocess/JSON-RPC smoke test).
  - `.planning/README.md` and `ARCHITECTURE.md` reference
    `cargo run --bin check_source_limits --locked` instead of the prior
    shell script; CI's name tags for gate steps are unchanged.
  - 32 unit tests live next to the algorithms they cover (token-aware
    state machine, attribute parser, criterion-estimate parsing,
    serde_json data merge).
  - The vendor/ tree, audit-prompt.txt, and message-history.json remain
    in the working tree untracked — out of scope for this decision and
    parked from prior turns.
- **Rationale**: the project's own principle — "build, test, benchmark,
  packaging checks, smoke tests, maintenance utilities, and internal
  tooling are all Rust" — is now literally true at the file level. No
  language-needs-to-be-installed requirement beyond `cargo` + the
  pinned toolchain. The prior `bash`/`python3` paths were an
  accidental dev-tooling leak of an older scaffold.
- **Consequences**: built and tested locally with the existing 1.97.1
  toolchain; full gate is green (fmt, clippy `-D warnings`, source-limits,
  no-duplicates, 421 tests including bin UTs, no regressions on the
  386-test lib suite). The single substituted dependency is the same
  `serde_json` already in `[dependencies]`; no new crates. CI's
  `ubuntu-latest` image needs no Python interpreter.
