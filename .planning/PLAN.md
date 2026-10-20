# SlugAudit Plan

This is the single active plan. Bugs, gaps, and pending changes belong here,
as a phase or in the open-debt list. Design and invariants live in
[`ARCHITECTURE.md`](ARCHITECTURE.md); decisions live in
[`archive/DECISIONS.md`](archive/DECISIONS.md); the operator runbook is
[`../docs/TROUBLESHOOTING.md`](../docs/TROUBLESHOOTING.md).

Every item carries an explicit state: **landed**, **in progress**,
**not started**, **still open**, **resolved**, or **dropped because …**. An
item with no state reads as "unknown" to the next reader, which is worse than
"known bad". Checkable claims carry a commit hash, a path, a command, or a
date.

The six audit records this plan refers to are copied into the repository so
they survive `/tmp` cleanup. They are new 2026-10-19 records; nothing is
restored from the deleted 2026-10-18 planning paths:

- `.planning/archive/audit_2026-10-19_report.md` — the strict audit of `main`
  at `26ecdb4`: 259/259 `.rs` files (50,325 lines), 2,550 confirmed
  violations.
- `.planning/archive/audit_2026-10-19_ledger.md` — the full verified
  violation ledger (2,550 lines, one per confirmed violation).
- `.planning/archive/audit_2026-10-19_wave_report.md` — the wave-1
  integrated report: commit map, findings fixed, public-schema changes,
  accepted unreachable arms, and the final gate at `fe923a7`.
- `.planning/archive/audit_2026-10-19_wave2_report.md` — the wave-2
  integration and final-gate report: `main` at `6728735`, all gates pass.
- `.planning/archive/audit_2026-10-19_wave3_report.md` — the wave-3 R4
  maintainability report: all 40 production entries split, 21 new modules,
  and the integrated gate at `2dbd703` + `d669848`.
- `.planning/archive/audit_2026-10-19_wave4_report.md` — the wave-4 R8
  public-contract report: `missing_docs` 241 → 0, the external-use decisions,
  the notices render, and the final gate at `4b765e1`.

The durable copies are the source of record. A later claim that cites a
`/tmp` path instead of one of these files is not durable.

## Current state (2026-10-19)

- `main` at `4b765e1` after wave 4 (`3dc9e42..976dc2e`, plus the doc-link fix
  `27ae582` and the surface re-record `cf1475e`). Wave 3 landed at `2dbd703`
  (`0374d60..2dbd703`, module map `d669848`); wave 2 landed at `6728735`;
  wave 1 at `fe923a7`; the base before wave 2 was `6241b43`.
- Crate `slugaudit-mcp-rust` `1.0.10`, edition 2024, `publish = false`,
  licensed by `LICENSE` (PolyForm Noncommercial 1.0.0 plus the SlugAudit
  Tool-Use Additional Permission).
- Tests: **1,176 passed, 0 failed, 1 ignored** on the integrated wave-4 tree
  (1,174 after wave 2; 1,156 after wave 1; 1,030 at the strict audit).
- Line coverage: **93.27%** (11,661 / 12,503), gate 83% — the integrated
  wave-3 and wave-4 measurement. Wave 2 measured 93.10%; wave 1 measured
  92.85%; the strict audit measured 90.26%.
- `check_docs_drift` and `check_mcp_surface` **pass** on `4b765e1`; the MCP
  surface was re-recorded at `cf1475e` (8 tools, 8 resources).

## Phase 1 — 2026-10-18 audit and remediation

State: **closed as an audit round; residue below is still open**.

The 2026-10-18 full code-quality audit was recorded at `7229ebd`. The
remediation round that followed landed between `7229ebd` and `26ecdb4`:

- Query and structure read paths: byte-paged structure results (`76a62cd`),
  oversized-cell clipping plus `line_count`/`line_range` (`115a7a3`), a 64 KiB
  default page with an agent override (`f8c13d4`), and removal of `query`'s
  500-row cap (`ddded2b`). Pages are now cut by `max_query_response_bytes`
  only; the agent writes its own `LIMIT`.
- Indexing and graph: one `LineIndex` per file (`d50ef26`), exclusion of only
  the runtime directory by path component with unknown file size stored as
  SQL NULL (`0492875`), and rejection of Python relative imports that climb
  past the project root (`4caa353`).
- Failure paths and bounds: one warn line for a stat-failed skip (`e219f90`),
  the remaining silent `Err(_)` arms failed or logged (`35f52be`), and the
  gate/test surfaces fail closed on unreadable inputs (`f3f7d25`, `5b35572`).
- Close-out: `7fcffd8` (structure line index, docs, surface, plan states) and
  `26ecdb4`, whose subject records the ledger at that point as **441 open /
  28 handled**.

Still open:

- The 2026-10-18 backlog ledger itself was deleted at `6241b43` with the other
  superseded planning records. Its 441 open entries were not individually
  re-verified in wave 1; the 2026-10-19 strict audit is the fresh, complete
  inventory (259/259 files) and is treated as the current defect list, not as
  evidence that the older ledger was closed.
- Any item that still reproduces in the current tree is captured by the
  remaining-debt list below (R4/R5/R8), not by a reference to the deleted
  ledger.

## Phase 2 — Resource bounding

State: **landed for the read/import paths listed; one architectural decision
still open**.

Landed (2026-10-18/19, before wave 1):

- Bounded foreign-file reads: `util::read_file_capped_bytes` /
  `read_file_capped` with a stat check plus a bounded `take(max_bytes + 1)`
  read (`b199277`), and the `.gitignore` read capped at 1 MiB (`dd87904`).
  `MAX_FOREIGN_TEXT_BYTES = 1024 * 1024` in `src/util.rs`.
- Streaming hashing and a batched reconcile hash lookup (`94726ab`).
- `query` skips cloning rows that cannot fit the page (`4d50e18`).
- Watch scope bounded by `max_watch_entries` / `max_watch_depth`, with a
  truncated walk reported as `WatcherHealth::Partial`; SQLite values clamped
  to what the bundled engine can store (`7d87f04`), and the scope refreshed
  before a full verification (`a1147f9`).
- Wave 1 added manifest path-set paging and sample bounds (`0fdf1f4`) and
  batched stale-finding invalidation at 4,096 paths (`c1c6628`).

Still open:

- **A separate memory/in-flight bound for import.** `max_total_import_bytes`
  is a sampled-bytes work budget, not a process-memory bound: peak memory also
  includes in-flight samples, evidence payloads, and parse transients. The
  design of a real memory bound is a pending decision
  (`src/model/limits.rs`, `ARCHITECTURE.md` *Store and schema lifecycle*,
  2026-10-19).
- Watch-scope caps are process/env limits (`SLUGAUDIT_MAX_WATCH_ENTRIES`,
  `SLUGAUDIT_MAX_WATCH_DEPTH`); the watcher is registered before a project
  config is loaded, so `config.json` cannot override them. Accepted, not a
  bug (`ARCHITECTURE.md` *Publish and revision model*).

## Phase 3 — Wave 1 (2026-10-19 production hardening)

State: **landed at `fe923a7`; residue listed below is still open**.

- **55 confirmed production findings fixed** from the strict audit:
  R1 3, R2 1, R6 22, R7 5, R10 3, R12 21. The per-package map is in
  `.planning/archive/audit_2026-10-19_wave_report.md`.
- **Top-10 item #10 fixed**: the CAS baseline in `src/sync/manager_health.rs`
  is re-read on every barrier iteration, so a change during iteration 1 is
  reconciled in the same call instead of aborting with `StaleBaseline` and
  marking the watcher `Desynced`.
- **Failure-path coverage**: 107 in-scope R5 entries were targeted; 126 tests
  were added (1,030 → 1,156). The three dead variants
  (`SyncError::Revision`, `SyncError::Read`, `DiscoveryError::Walk`) were
  reported, not tested.
- **Coverage**: 90.26% → 92.85% (11,212 / 12,075; gate 83%).
- **Public schema changes** (documented in the wave report): `finding_read`
  paging fields, `ReportResponse.completeness.unrepresentable_path_count`,
  `HealthPhase::Unknown`, `ActivationError::DatabaseOperation`,
  `ConnectError::AddFailedNoOutput`, `DisconnectOutcome`, and several schema
  descriptions.

Wave 1 residue and its wave-2 disposition:

- The unreachable/untestable arms listed under *NEEDS-DECISION* in
  `.planning/archive/audit_2026-10-19_wave_report.md` were dispositioned by
  wave 2 at `ed58a4b`: the testable ones now have tests through pure helpers,
  `SyncError::Revision` and `SyncError::Read` were removed,
  `DiscoveryError::Walk` was retained as live code, and the remaining
  acceptances are listed under *Accepted arms and discarded variants* below.
- `report` counted non-UTF-8 paths with a bounded discovery walk on every
  call. Wave 2 replaced the walk with a persisted count (`b19f4a0`); see
  *Phase 4*.
- `check_docs_drift` and `check_mcp_surface` failed because their inputs had
  been deleted. Wave 2 authored the docs (`7a782de`, `326ec9a`) and recorded
  the surface (`5d77856`); both gates now pass.

## Phase 4 — Wave 2 (landed)

All six items are **landed** on `main`; the final gate below was run on the
integrated tree at `6728735`.

| Item | Package | State | Evidence |
|---|---|---|---|
| Canonical docs authored fresh (this file, `archive/DECISIONS.md`, `docs/CLIENTS.md`, `docs/TROUBLESHOOTING.md`, `docs/RELEASING.md`, `docs/about.toml`, `docs/third-party-notices.hbs`) | wave2-docs | landed | `7a782de`, plus `326ec9a` correcting AGENTS.md's parser-pack path; this record adds the durable audit copies |
| MCP surface snapshot recorded from the live surface | wave2-surface | landed | `5d77856`: 8 tools, 8 resources, including the `finding_read` paging fields, the persisted `unrepresentable_path_count` description, and `HealthPhase::unknown`; `check_mcp_surface` PASS |
| Persist the non-UTF-8 path count at sync; `report` reads it from the database instead of walking the filesystem | wave2-report | landed | `b19f4a0`: `unrepresentable_paths (path BLOB PRIMARY KEY)`, schema 7 → 8, full-publish and stat-sweep refresh; tests cover add → 1 → remove → 0 through full and watcher-backed syncs |
| CLI writes fail gracefully on a closed pipe (`BrokenPipe` is a clean exit; no panic) | wave2-cli | landed | `bac3d0a` + `1b2b671`: `cli::out` / `cli::err` / `write_to` / `is_broken_pipe`; `menu \| head -c 10` exits 0 with empty stderr, `help >/dev/full` exits 1 with the I/O error |
| Remaining unreachable failure arms tested through a clean refactor, or explicitly accepted/removed; no production-only test hooks, no lint suppressions | wave2-failure | landed | `ed58a4b`: resolutions and acceptances under *Accepted arms and discarded variants* below |
| Fresh performance baseline recorded after the code changes | lead, after integration | landed | `6728735`: 12 benchmarks (discovery 4, parsing 4, search 4) recorded to `.planning/archive/perf_baseline.json`; comparison 0.98x–1.14x, threshold 20% |

### Final gate (2026-10-19, `main` at `6728735`)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass |
| tests | **1,174 passed, 0 failed, 1 ignored** |
| `cargo deny check` | pass (advisories/bans/licenses/sources; one unmatched license exception warning) |
| `cargo machete` | pass |
| coverage | **93.10%** line coverage (11,341 / 12,181; gate 83%) |
| `check_source_limits` | pass |
| `check_no_duplicates` | pass |
| `check_producer_coverage` | pass |
| `check_governance` | pass |
| `check_version_consistency` | pass |
| `check_docs_drift` | **pass** |
| `check_mcp_surface` | **pass** |
| `check_performance` | **pass** (0.98x–1.14x, threshold 20%) |

The full wave-2 account is in
`.planning/archive/audit_2026-10-19_wave2_report.md`.

## Phase 5 — Wave 3 (landed)

State: **landed**; the integrated full gate passed at `2dbd703` + `d669848`.

- **All 40 production R4 entries fixed** from the strict audit: 18 files above
  200 code lines split to ≤200, 17 functions above 50 lines split to ≤50, and
  5 functions with cyclomatic complexity above 15 reduced to ≤15. The
  production `slugaudit-line-exception` annotations were removed and
  `check_source_limits` passes without them. One stale annotation comment
  remains at `src/tools/health.rs:33` even though that file is now 108 code
  lines and passes without it; removing it is a source cleanup follow-up, not
  a gate failure.
- **21 new internal modules**, each listed in `MODULE_MAP.md`:
  `connect/config_file.rs`, `connect/registry_types.rs`,
  `evidence/bindings.rs`, `graph/rust_paths.rs`, `project/gitignore.rs`,
  `server/handlers.rs`, `store/connection_guards.rs`, `sync/hash_io.rs`,
  `sync/manager_health_support.rs`, `tools/finding_insert.rs`,
  `tools/health_phase.rs`, `tools/report_envelope.rs`,
  `tools/report_sections.rs`, `tools/tool_counters.rs`,
  `util/capped_read.rs`, `util/style.rs`, `util/time.rs`, `watch/errors.rs`,
  `watch/manager/events.rs`, `watch/manager/scopes.rs`, and
  `watch/scope_apply.rs`.
- **Seven packages**, one commit each: `refactor(sync): split publish and
  reconcile`, `refactor(sync): split core hash and health`,
  `refactor(watch/store)`, `refactor(connect/project)`,
  `refactor(tools/query)`, `refactor(tools): split finding, report and
  health`, and `refactor(server): split server, evidence and util`. The full
  library suite was green in each package; the refactors were structural, and
  the tool request/response schemas are unchanged.
- **Integrated gate (2026-10-19, `2dbd703` + `d669848`)**: fmt, clippy, deny,
  machete, source limits, duplicate checks, producer coverage, governance, and
  version pass; **1,176 tests passed, 0 failed, 1 ignored**; coverage
  **93.27%** (11,661 / 12,503; gate 83%); performance pass at 0.97x–1.06x
  against the 20% threshold; **0 production files over 200 code lines**; docs
  drift and `check_mcp_surface` pass. The only remaining clippy
  `too_many_lines` warning is a test function in
  `tools/report_completeness.rs`, not production code.
- **Mutation scope**: now six files. `src/sync/hash_io.rs` was added because
  `hash_file` and `stat_identity` moved there from `hash.rs`;
  `src/sync/revision_support.rs` was added in wave 4 because
  `invalidate_stale_findings` moved there from `revision.rs`. The other four
  files are unchanged. Recorded in `.planning/archive/DECISIONS.md`
  (2026-10-19), `.github/workflows/quality.yml` (`4b765e1`), and `AGENTS.md`.

## Phase 6 — Wave 4 (landed)

State: **landed at `3dc9e42..976dc2e`** (doc-link fix `27ae582`, surface
re-record `cf1475e`); the integrated full gate passed on `main` at `4b765e1`.

- **All 258 production R8 entries closed**: 241 current `missing_docs` sites
  were documented (`241 → 0`), and six external-use items were made
  crate-private, test-only, or removed from the public surface.
  `cargo doc --no-deps --lib` reports **0 warnings** after the final 10
  intra-doc link fixes.
- **External-use decisions**: `cli::USAGE`,
  `SlugAuditServer::with_manager`, and `ErrorCode::as_str` → `pub(crate)`;
  `LineIndex::line` → `pub(crate)` plus `#[cfg(test)]`;
  `WatchScopeLimit` → `pub(crate)` and removed from the watch re-export, with
  `WatchScope::limit` crate-private; `ErrorCode::ALL` → `#[cfg(test)]
  pub(crate)`, so the `ARCHITECTURE.md` comparison sentence now names the
  test-local enumeration.
- **Surface**: the final `.planning/archive/mcp_surface.json` was re-recorded
  from the integrated tree at `cf1475e` (8 tools, 8 resources), including the
  doc comments that become JSON-Schema descriptions.
- **Release notices**: a full `cargo-about 0.9.2` render through the exact
  workflow command produced 6,558 lines / 340,298 bytes with exit 0 and no
  warnings. The render fixed HTML escaping (license names use `{{{name}}}`,
  no `&quot;` entities), removed the machine-specific `source_path` block,
  and recorded the upstream `ring` 0.17.14 ISC
  source-header/code-fragment limitation in `docs/RELEASING.md` as a tool
  limitation, not a template defect.
- The stale source-limit annotation comment in `src/tools/health.rs` was
  removed.

### Final gate (2026-10-19, `main` at `4b765e1`)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass |
| tests | **1,176 passed, 0 failed, 1 ignored** |
| `cargo deny check` | pass |
| `cargo machete` | pass |
| coverage | **93.27%** line coverage (11,661 / 12,503; gate 83%) |
| `check_source_limits` | pass (0 production files over 200 code lines) |
| `check_no_duplicates` | pass |
| `check_producer_coverage` | pass |
| `check_governance` | pass |
| `check_version_consistency` | pass |
| `check_docs_drift` | **pass** |
| `check_mcp_surface` | **pass** |
| `check_performance` | **pass** (0.97x–1.09x, threshold 20%) |
| `missing_docs` / `cargo doc --no-deps --lib` | **0 warnings** |

The full wave-4 account is in
`.planning/archive/audit_2026-10-19_wave4_report.md`.

## Phase 7 — Wave 5 (in progress)

State: **in progress** — a fresh audit of the wave-4 tree. No result is
claimed here; this entry records only that the wave has started.

## Remaining P1 debt (still open)

These are production entries from
`.planning/archive/audit_2026-10-19_report.md` that wave 1 did not close. They
are recorded here so a later wave does not rediscover them from scratch.

- **R4 — functions/files over size targets: 109 entries, 40 production.**
  **Resolved for production at wave 3** (`0374d60..2dbd703`): all 40
  production entries (18 files > 200 code lines, 17 functions > 50 lines, 5
  functions with CC > 15) were split. The other 69 entries are 60
  test/bench/fuzz files and 9 dev-gate files; test files auto-pass to 500 code
  lines by contract, and the dev-gate entries were outside wave 3's production
  scope.
- **R8 — public docs / external use: 258 production entries.** **Resolved at
  wave 4** (`3dc9e42..976dc2e`, `27ae582`): the 241 `missing_docs` sites were
  documented to 0, `cargo doc --no-deps --lib` reports 0 warnings, and the
  six external-use items were made crate-private, test-only, or removed from
  the public surface. The audit count also included public items with no
  external reference (dead public API), which are no longer public.
- **R5 — non-boundary failure-path tests: 142 entries (110 production, 32
  dev-gate) at the strict audit.** Wave 1 targeted 107 in-scope entries; wave
  2 closed the boundary/unreachable-arm subset (`ed58a4b`). The remaining
  **non-boundary R5** entries and the two still-accepted finding arms under
  *Accepted arms and discarded variants* are **still open**.

## Accepted arms and discarded variants

Disposition from `ed58a4b` (wave 2), so a later reader does not re-open an
arm that was already decided. "Accepted" means the arm stays in production
code as defensive handling and is documented at the site; it is not an
untested path waiting for a test hook.

- **Removed (provably unused):** `SyncError::Revision` and
  `SyncError::Read`.
- **Retained (live, not dead):** `DiscoveryError::Walk`. It is constructed in
  `src/sync/discovery.rs`'s skip path (`DiscoveryError::Walk(error)`), so the
  wave-1 premise that it was unreachable was wrong. Do not remove it.
- **Accepted as unreachable/documented:** the connection `Configure` arms for
  `busy_timeout` and function registration in `src/store/connection.rs`.
  They fail only on `SQLITE_NOMEM`/`SQLITE_MISUSE` from a NULL or invalid
  handle; the safe rusqlite API cannot produce either hermetically, and the
  5-second timeout behavior itself remains tested.
- **Still accepted from wave 1:** the `finding` u32 line-count conversion
  (`src/tools/finding.rs`) and the `finding_read` query-map bind failure
  (`src/tools/finding_read.rs`). Both are unreachable for storable content /
  valid bound parameters; wave 2 did not revisit them.
- **Tested through pure helpers, not production hooks:** install's
  `CurrentExe`/`Clock`, connect's `BinaryPath`, prompt flush, and update's
  platform predicate (`ensure_platform_supported`).
- **Tested through clean refactors:** the stale-metadata discard path, the
  netfs positive rejection (`reject_if_network(path, on_network)`), the
  context read-transaction start, and project-control's discard/reopen path
  (its serialize arm was removed by constructing the value directly).

## Accepted and held decisions

These are deliberate. Reversing one needs a new entry in
`archive/DECISIONS.md`, not a drive-by change.

- Evidence is not judgment: no risk scores, severity, ranking, or
  "priorities" in any output. Findings are agent-authored.
- The database is disposable derived data; schema changes are
  discard-and-rebuild, never migrated (`src/store/migrations.rs`).
- Findings are session-scoped and bound to a content hash.
- stdio is the only transport; no network listener, health endpoint, or
  metrics endpoint.
- The path-logging policy (2026-10-16): correlation logs carry
  BLAKE3-derived path ids; raw paths are allowed only in one-shot diagnostics
  beside the file being operated on.
- `tree-sitter-language-pack` runtime grammar downloads are the accepted
  grammar-distribution path; the 2026-10-18 static bundle was reverted on
  2026-10-19.
- The coverage gate is 83%, calibrated against measurement, not aspirational.
- Mutation testing covers exactly `sync/revision.rs`,
  `sync/revision_support.rs`, `sync/publish.rs`, `sync/hash.rs`,
  `sync/hash_io.rs`, and `tools/context.rs`; widening beyond that is a
  CI-minute decision. `hash_io.rs` carries `hash_file` and `stat_identity`,
  moved out of `hash.rs` by the wave-3 split, and `revision_support.rs`
  carries `invalidate_stale_findings`, moved out of `revision.rs` by the
  wave-4 size fix (2026-10-19), so this is the same correctness core, not a
  wider one.
- Tests are exempt from the production panic ban: a panic is the failure
  signal there.
- The 2026-10-19 strict audit counts test/bench/fuzz R1/R2 entries, but the
  crate's `cfg(test)` exemption is intentional; those counts are not a defect
  list to burn down mechanically.

## Documentation policy

State: **landed (wave 2)** — authored at `7a782de`, with the AGENTS.md
parser-pack path corrected at `326ec9a`; this record adds the four durable
audit copies under `.planning/archive/`.

The canonical doc set is fixed by
`src/bin/check_docs_drift/canonical_docs.rs`:

```
README.md
AGENTS.md
.planning/ARCHITECTURE.md
.planning/MODULE_MAP.md
.planning/PLAN.md
.planning/archive/DECISIONS.md
docs/CLIENTS.md
docs/TROUBLESHOOTING.md
```

The previous planning record was deleted deliberately at `6241b43`. The
surviving canonical documents are authored from current behavior; no deleted
document is restored or copied from git history. `check_docs_drift` passes on
the integrated tree; it enforces existence, the module-map paths, and the
`*_tests.rs` count claim. The release docs (`docs/RELEASING.md`,
`docs/about.toml`, `docs/third-party-notices.hbs`) are consumed by
`.github/workflows/release.yml` and are not part of the drift gate's checked
set. The six `audit_2026-10-19_*` files in `.planning/archive/` are
historical evidence copies, not canonical docs, and are not read by the gate.
