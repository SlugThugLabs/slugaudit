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

The eight audit records this plan refers to are copied into the repository so
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
- `.planning/archive/audit_2026-10-19_final_report.md` — the wave-5
  verification audit of `main` at `66536c0` (code at `4b765e1`): 282/282 files
  (57,449 lines) read by 12 chunk auditors, 56 confirmed P0/P1, the fixes,
  the accepted P2 residue, and the post-fix final gate.
- `.planning/archive/audit_2026-10-19_final_ledger.md` — the verified wave-5
  ledger: 56 confirmed P0/P1 rows (12 P0, 44 P1), the 9 duplicate rows merged
  into them, the 9 suspected misses (recorded, not counted), and the per-rule
  and per-scope counts.

The durable copies are the source of record. A later claim that cites a
`/tmp` path instead of one of these files is not durable.

## Current state (2026-10-28)

- `main` carries Phase 8 Waves A, B and C and the wave-1 remediation (P1, P3, P4, P5, P6/6b, P7, P8), merged as `4a70544` (pushed; not tagged). Crate `slugaudit-mcp-rust` **1.1.0** (`CHANGELOG.md` `[1.1.0]`), 9 tools (adds `mechanic`), one shared `page` contract, 1-based lines, schema version 11, `[profile.release]` strip + thin LTO + codegen-units 1 (release binary 18.7 MB -> 12.4 MB).
- Gate at `f43dc9b` (Phase 8 close): 12/12 README *Development* gates, 1,507 tests passed, 0 failed, coverage 93.28%; release-binary stdio end-to-end 12/12. `f973bc6` (test-only flake fix) follows; the final gate re-ran green at `d8cf8e4` (recorded in the Phase 8 header). Wave-1 evidence and outcome: `.planning/archive/remediation_2026-10-28_wave1.md`.
- Wave 2 (2026-10-28): activation marker + rem2 branches; outcome in `.planning/archive/remediation_2026-10-28_wave2.md`.
- Branch `wave-c-c4` and worktree `/home/slugaudit-wt/c4` (C4, integrated as `ef3b98c`) and branch `wave-a-lane-y` were kept for the owner to delete (2026-10-26); state: **still open** until the wave-2 housekeeping report says otherwise.

### Previous state (2026-10-19)

- `main` at `2a65a85` after wave 5 (`66536c0..2a65a85`, the 56-fix set plus
  the integration fixes `730699f` and `2a65a85`). Wave 4 landed at `4b765e1`
  (`3dc9e42..976dc2e`, plus the doc-link fix `27ae582` and the surface
  re-record `cf1475e`); wave 3 at `2dbd703` (`0374d60..2dbd703`, module map
  `d669848`); wave 2 at `6728735`; wave 1 at `fe923a7`; the base before
  wave 2 was `6241b43`.
- Crate `slugaudit-mcp-rust` `1.0.10`, edition 2024, `publish = false`,
  licensed by `LICENSE` (PolyForm Noncommercial 1.0.0 plus the SlugAudit
  Tool-Use Additional Permission).
- Tests: **1,246 passed, 0 failed, 1 ignored** on the integrated wave-5 tree
  (1,176 after wave 4; 1,174 after wave 2; 1,156 after wave 1; 1,030 at the
  strict audit).
- Line coverage: **93.56%** (12,005 / 12,831), gate 83% — the integrated
  wave-5 measurement. Wave 4 measured 93.27%; wave 2 measured 93.10%;
  wave 1 measured 92.85%; the strict audit measured 90.26%.
- `check_docs_drift` and `check_mcp_surface` **pass** on `2a65a85`; the MCP
  surface snapshot was updated with the wave-5 health and `project_control`
  schema changes at `4f3760d` (8 tools, 8 resources).

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

## Phase 7 — Wave 5 (landed)

State: **landed at `66536c0..2a65a85`** (the 56-fix set plus the integration
fixes `730699f` and `2a65a85`). The wave-5 audit read the wave-4 tree at
`66536c0` (code at `4b765e1`; later commits were docs only).

### Audit result

- **12 independent chunk auditors read 282 of 282 files (57,449 lines)**, and
  a verification pass reopened every cited line and enclosing function.
- **56 confirmed P0/P1 (12 P0, 44 P1)** from 65 author candidates (17 P0, 48
  P1): **9 duplicate rows merged** into confirmed rows, **0 dropped** as
  false, misstated, or already covered, and **9 suspected misses** recorded
  separately but not counted. Scope: 49 production, 2 dev-gate, 5 test.
- **All 56 confirmed entries fixed** by the wave-5 commit set. The P0 set:
  fail-closed sweep/discovery on unreadable trees and malformed ignore files;
  duplicate top-level SQL column names refused; the notify-callback deadlock
  removed; NFS-root detection; false disconnect success and 0600/symlink
  client-config writes; cross-session finding purge moved to session start;
  path redaction in ignore-rule, sweep, manager open/recovery, reconcile/drain,
  and parser-load logs; the `finding_read` offset test. The P1 set: health and
  structure error typing plus the unreadable-DB phase; report operation
  context; `project_control` `ImportReport` fields
  (modified/deleted/skipped); gitignore write durability and FIFO/symlink
  handling; query-clip
  continuation; sweep decomposition and discovery `entry.error()`; ignore
  parent/hidden consistency; watch scope bound, dead state APIs, poison test,
  lock labels, deterministic event tests; model/graph dead API removal and
  Python relative imports; update `run_update` shipped-platform tests; docs
  and source-limit gate parsing; test fixture leak, finding caps, redaction
  control lines, connect I/O/stat assertions.
- The per-row evidence, failure scenarios, existing-test gaps, and rule/scope
  counts are in `.planning/archive/audit_2026-10-19_final_ledger.md`; the
  report is `.planning/archive/audit_2026-10-19_final_report.md`.

### Accepted design decisions

- **Watch scope is non-recursive, with the sweep as the coverage guarantee.**
  The initial project watch and every directory the scope walk includes are
  registered with `RecursiveMode::NonRecursive`; the walk already enumerates
  every indexable directory, so the number of kernel watches is bounded by
  `max_watch_entries` instead of by the whole subtree (ignored trees
  included). A directory created after the walk marks the scope dirty and is
  registered on the next scope refresh; files created inside it before that
  are caught by the per-call stat sweep. A changed watch set forces
  `NeedsVerification`, so the window is verified, not assumed.
- **Ignore rules are project-scoped.** The walker uses `parents(false)`:
  `.gitignore`, `.ignore`, and `.git/info/exclude` files are read at or below
  the project root, and ancestor ignore files above the root are not honored.
  This is the rule set `IgnoreRules::build`, discovery, the watch-scope walk,
  the sweep, and reconcile all share, so the full and incremental paths cannot
  disagree about what is indexed.
- **`ImportReport.skipped` is a count, not a list.** `project_control` now
  reports `added`, `modified`, `deleted`, `unchanged`, `skipped`, and
  `revision_id`; `added` includes files the import then skipped, and the
  per-reason breakdown stays in `report.completeness`. An unbounded list would
  be cut by the response byte budget; a count is quotable as-is.
- **The connection guards stay accepted as documented.** The `Configure` arms
  for `busy_timeout` and function registration remain accepted as unreachable
  without an invalid handle or OOM; the one-shot security rejections in
  `connection_guards.rs` keep raw paths under the 2026-10-16 one-shot
  diagnostic carve-out.

### Remaining P2 follow-ups

None of these is a confirmed P0/P1 defect; they are recorded so a later wave
does not rediscover them from scratch.

- **`connect_cli` argv is still lossy.** State: **resolved** in wave 6 (`8467cfd`, branch `wave6-connect-loop`).
  `add_args` validates `binary.to_str()` and returns `ConnectError::BinaryPath`
  on non-UTF-8 paths before running CLI clients, refusing the path before the
  client can be executed and preventing U+FFFD substitution.
- **`write_atomic` does not fsync before rename.** State: **resolved** in wave 6 (`1c40a1d`, branch `wave6-connect-loop`).
  `write_atomic_with` performs `file.sync_all()` before rename and a directory
  sync after rename, warning if directory sync fails.
- **Connect hardening (FIFO safety, command quoting, relaying client output, error retention).** State: **resolved** in wave 6
  (`3625c83`, `a7db8d4`, `2760c01`, and follow-up fixes on branch `wave6-connect-loop`).
  Refuses non-regular/FIFO configs via symlink metadata check before opening or reading,
  quotes displayed commands, relays client failure output without truncation, disconnects
  without a home via client CLI remove, retains source errors on output print failure,
  and secures FIFO test concurrency via `TEST_ENV_LOCK` and absolute `mkfifo` binary discovery.
- **The ignore-error mapper is duplicated.** State: **resolved** on `main`.
  `src/ignore_rules/build.rs` now shares `crate::watch::errors::ignore_error_kind`
  directly, eliminating duplicate error variant mapping and dropping `build.rs` to 150 code lines.
- **Test-only and dev-gate P2 notes.** State: **in progress** on `main`. `canonical_docs.rs`
  module doc updated to include `MODULE_MAP.md`; `limits.rs` doc corrected for oversized row clipping;
  remaining dev-gate test items recorded as low-priority follow-ups.
- **The 9 suspected misses stay recorded but uncounted.** They are listed in
  the final ledger. The `ImportReport` deleted/skipped gap was closed by wave 5;
  the client-config FIFO read was **resolved** in wave 6 (`a7db8d4`);
  lossless non-UTF-8 sidecar cleanup (`connection.rs`) and progress completion
  emission on all exit paths (`manager_health.rs`) were **resolved** on `main`;
  remaining items (line-index amplification and related size notes) remain documented P2s.

### Final gate (2026-10-19, `main` at `2a65a85`)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass |
| tests | **1,246 passed, 0 failed, 1 ignored** |
| `cargo deny check` | pass (one unmatched license-exception warning) |
| `cargo machete` | pass |
| coverage | **93.56%** line coverage (12,005 / 12,831; gate 83%) |
| `check_source_limits` | pass (0 production files over 200 code lines; `manager_orchestration.rs` at 197) |
| `check_no_duplicates` | pass |
| `check_producer_coverage` | pass |
| `check_governance` | pass |
| `check_version_consistency` | pass |
| `check_docs_drift` | **pass** |
| `check_mcp_surface` | **pass** |
| `check_performance` | **pass** (0.99x–1.02x, threshold 20%) |
| `missing_docs` / `cargo doc --no-deps --lib` | **0 warnings** |
| `cargo crap --min 0 --path src` | 0 production functions with CC > 15 |

## Phase 8 — 2026-10-25 deep-dive remediation and auditor-owned mechanics

State (2026-10-28): **wave 1 landed** (P1, P3, P4, P5, P6/6b, P7, P8; merged as `4a70544` on `main`, pushed; report `.planning/archive/remediation_2026-10-28_wave1.md`). Wave 2 (2026-10-28): activation marker + rem2 branches; outcome in `.planning/archive/remediation_2026-10-28_wave2.md`.

State: **Wave A landed** (2026-10-25, `7dc6410..35aabea`, gate green at `35aabea` (12/12 README *Development* gates, 1317 tests, 93.96% coverage); Wave B and Wave C landed 2026-10-26 (Wave C: C1-C6, see below), gate green at `345618f` (12/12 README *Development* gates, all test blocks 0 failed, 93.35% coverage; release-binary stdio end-to-end 8/8); Wave C gate green at `d8cf8e4` (12/12 README *Development* gates, all test blocks 0 failed, 93.29% coverage, zero-limit tests 10/10 runs clean; release-binary 1.1.0 stdio end-to-end 12/12 at `f43dc9b`)). Authored 2026-10-25 by the architect from the deep dive;
Lead executes on branch `fix/deep-dive-2026-10-25`, one commit per item, no
push). Evidence: `.planning/archive/audit_2026-10-25_deep_dive.md` (defects
D1–D9, `path:line` at `c3fc7f8`). Each item lists its Definition of Done; an
item is **landed** only when its DoD check passes and the full gate list in
*Development* (README) is green on the integrated tree.

Product intent recorded this session (owner, 2026-10-25): the tool is for an
LLM in a harness (Claude Code and peers) with cwd = the project; the database
is throwaway and session-scoped so one model's view never clouds another's;
the tool ships counting primitives, never a checklist, because a shipped
checklist makes a model think "ran tool, audit done". What counts as a signal
in a codebase is the auditor's decision, recorded by the auditor (the model)
as mechanics it registers for the session or persists to the project config.

### Wave A — safety and correctness (parallel; one owner per file set)

- **A1 — PRAGMA at prepare time and engine limits (D1).** State: **landed** — `7dc6410` (+ `44626bb` guard tests moved and docs, `1052963` rebuild test reads `user_version` raw). Deviations: introspection-pragma allowlist keeps `pragma_table_info` working; `SQLITE_LIMIT_LENGTH` = max(`max_query_value_bytes`, 3 × `max_file_bytes`) per `query` call, so the amplifier is bounded but can still reach ~1.5 GB (estimated, not measured) on the audit profile — **still open** as a residual risk pending an owner decision on capping stored content.
  Deny `AuthAction::Pragma` in the read-only connection's authorizer
  (`src/store/connection_guards.rs`), keeping `pragma_table_info`/`pragma_*`
  table-valued reads working or documenting their loss in INSTRUCTIONS; set
  `SQLITE_LIMIT_LENGTH` (and `SQLITE_LIMIT_SQL_LENGTH`) on the read-only
  connection from `max_query_value_bytes`; replace the misleading
  "only comments" message for a refused non-SELECT. DoD: a test that runs
  `PRAGMA hard_heap_limit=1` through `query`, asserts the refusal, then opens a
  second connection and reads `PRAGMA hard_heap_limit` = 0 and a `report`
  succeeds; a test that the `zeroblob/hex/replace` amplifier fails with a
  bounded error instead of allocating.
- **A2 — 1-based line numbers on every surface (D2).** State: **landed** — `a1427c2` (schema version 9; conversion in `evidence/sql.rs::to_row` and `structure_page.rs`). Release-binary repro: `leads` 3/4, `structure` 1–5, evidence rows agree with `cat -n`.
  Keep `model::span` zero-based internally; convert once at the evidence
  write boundary (`src/evidence/normalize.rs`) so `evidence.start_line/end_line`
  are 1-based, and at the `structure` response builder; bump the schema
  version so existing databases rebuild; document "all line numbers are
  1-based, inclusive" in INSTRUCTIONS, `schema.sql` column comments and
  `evidence_payload_keys.md`. DoD: an end-to-end test over a fixture file
  asserts `leads`, `query` evidence rows, `structure`, `finding` echo and
  `line_range()` all agree with `cat -n`; `check_mcp_surface --record` if a
  description changed.
- **A3 — symlink-safe writes (D3).** State: **landed** — `66d1ac3`. An in-project `.gitignore` symlink is now refused too (deviation). Untested: the `Io` → internal-error arm at the `project_control` boundary. Before writing
  `config.json` or editing `.gitignore`, refuse when `symlink_metadata` of
  the target is a symlink or a non-regular file, with a typed error and remedy;
  no canonicalise-then-write. DoD: tests for `config.json -> outside` and
  `.gitignore -> outside` assert the target is untouched and the error names
  the path.
- **A4 — parse bounded by size and time (D4).** State: **landed** — `800630d` (+ `35aabea`). 50 MB case: 1.74 s, 170 MB peak RSS, `files_not_parsed = 1`. The sampler uses the pack's parse timeout (wraps the progress callback) bound to the one sync deadline; `files_not_parsed` excludes `language IS NULL`. **Still open:** no per-file parse budget, so a pathological file under 2 MiB that outlives `max_sync_wall_clock` fails every sync until edited; `Query::new` has no deadline. Add
  `max_parse_bytes` to `Limits`/profiles (default 2 MiB; env
  `SLUGAUDIT_MAX_PARSE_BYTES`; config override) distinct from
  `max_file_bytes`; a file above it is stored (subject to `max_file_bytes`)
  with `extraction_completeness = ContentOnly` and a reason
  `oversize_for_parse`, counted in `report.completeness`; every `leads` rows
  and counts response carries `files_not_parsed`; the sampler's and
  `structure`'s `parser.parse` run under tree-sitter's progress callback
  bound to the call deadline; `structure` checks the window before pushing a
  file. DoD: a 3 MiB generated Rust file indexes in < 2 s with
  `files_not_parsed = 1` visible in `leads` and `report`; the 50 MB case from
  the deep dive completes under 10 s with no SIGKILL.
- **A5 — prose is stored, not parsed (D5).** State: **landed** — `9cf6141` + `5823049` (+ `8749608`). Prose is stored as `language = 'prose'`, excluded from `parser_failure_*`, counted in `completeness.prose_files_not_parsed`; `structure` refuses prose. Classify
  Markdown, reStructuredText, plain text and similar prose as `prose`: content
  stored, no grammar run, no `Diagnostic` rows; override the pack's `.txt`
  → `vimdoc` mapping in `src/parse/language.rs`; structured config
  (JSON/YAML/TOML) stays parsed. DoD: indexing a project with a Markdown file
  containing an unterminated code fence yields 0 diagnostics; a YAML with a
  tab-indent error yields 1.
- **A6 — grammar download: honest and retried (D6).** State: **landed** — `e34e0ca` + `0ed7c30`. Deviation: `has_language` kept as the gate (`has_parser` never downloads); retries gated by a process-wide 10-minute fetch cool-down after the first version retried every call and published a revision per call offline. A black-holed network blocks the first call ~60 s (pack's fixed timeout).
  Use `has_parser` where availability is meant; map the pack's download error
  to `LoadFailed` so the file is re-attempted on the next sync even when its
  hash is unchanged; document in README *Install* and in INSTRUCTIONS that
  grammars are fetched lazily from the language pack's GitHub release on
  first use, where they are cached, and what happens offline. DoD: a test
  with the download forced to fail records `LoadFailed`, and a second sync
  with it allowed parses the file; docs drift gate passes.

### Wave B — value (after Wave A is integrated and green)

- **B0 — Wave A residue (architect, 2026-10-25).** State: **landed** — `c4635d3` (config-write Io arm tested via a clean refactor), `d1f93c8` (per-file `max_parse_wall_clock_ms`, default 5000; a timed-out file is `parse_timeout`, counted, and the sync continues; each bounded parse runs on a short-lived thread because the pack reuses a cancelled parser per thread and then panicked — cost ~0.11 ms/file, 10k-file soak 5.1 s -> 6.3 s), `4525da2` (`max_structure_query_bytes` 16 KiB on every profile, refused before `Query::new`), `2f61367` (0 refused for the three parse/structure limits; override warnings once per cause), `e1d7939` + `1f13c6a` (ignore/attribute dotfiles stored as prose; schema 10). Untested: the pack-panic -> `ParseFailed` mapping (no hermetic trigger).

- **B1 — call evidence for Python, JavaScript, TypeScript, TSX, Go, Java, C
  (D7).** State: **landed** — `79b7c7d` (schema 11). Embedded `examples/calls/<language>.scm` queries for the seven languages; Rust keeps its walker (a query cannot reach macro token trees); turbofish fixed (83 rows renamed, +7 turbofish calls inside macro arguments that were silently dropped); `covers` = languages actually read. Built-in counts on this repository's own sources unchanged. Replace the Rust-only node matching with a
  per-language table or embedded `.scm` call queries (one `@callee` capture),
  keeping the shared span/confidence/detail logic and the Rust macro re-parse;
  `covers` becomes the explicit set of languages a mechanic reads; fix the
  Rust turbofish fall-through. DoD: one fixture per language with a known
  call count, including decoys in comments and strings; `leads` on the Python
  fixture returns the expected non-zero count with `covers` listing python.
- **B2 — auditor-owned mechanics.** State: **landed** — `bc78af2` + `f4de900` (review BLOCK fixed: every `config.json` writer goes through one locked, atomic read-modify-write; `project_control on profile` no longer deletes persisted mechanics; built-in-only `leads` scans no custom entry; `mechanic remove {index}`; alias languages refused) + `345618f` (alias drift test, one direction only — the pack's alias table is private). New tool
  `mechanic` with `add` / `remove` / `list`: an entry is `{id, language,
  query, note}`; `query` is compiled on `add` and a bad one is refused with
  the compile error; `add` returns the first count immediately (same fields
  as a `leads` count). Session entries live in server memory and die with the
  process; `persist: true` writes the entry to a versioned `mechanics`
  section of `.planning/slugaudit/config.json`, re-read per call. `leads` and
  `report` carry every entry with `source: builtin | project_config | session`
  and the author's `note`, session entries listed first. Rules: an entry can
  never remove, rename or override a built-in; ids must not collide; an
  invalid persisted entry is listed in `report` with its error and counted
  as `mechanics_invalid`, never dropped. The first `report` of a session
  carries a short `mechanics` note stating the built-ins are counting
  primitives, how to register, and the lifetime. DoD: tests for add/count/
  remove, persist/reload, collision, invalid query, invalid persisted entry;
  `check_mcp_surface --record`; INSTRUCTIONS and README updated.

### Wave C — surface and release (after Wave B)

- **C1 — one paging contract (D8).** State: **landed** — `3390516` + `c27212a` (fuzz target/docs) + `0f6bd1c` (Luna review fixes); quinn PASS (one `$defs/Page` in five schemas; stale resume refused on query and finding_read). Builder opus (before the 2026-10-26 routing correction). A shared page
  struct (`returned`, `total`, `complete`, `next_offset`, `page_bytes`,
  `bytes_used`, `limited_by`, `revision_id`) used by `leads`, `query`,
  `structure`, `finding_read`; per-tool synonyms removed; `structure`'s
  second cursor documented beside it. DoD: the four output schemas share the
  struct; `check_mcp_surface --record`.
- **C2 — instructions and errors (D8).** State: **landed** — `41043ad` (INSTRUCTIONS 12,561 -> 5,474 chars; `health` behind `verbose`; `sql_error` and `unknown_language` codes; recipes split and LIKE-on-payload removed; the first-report `stale_revision` was still live — `verify_and_drain` returned the publish's revision after its own drain published a newer one — fixed and pinned by `report_first_call_tests`) + `9ed41f3` / `3919f5f` (Luna: `detail` object-only, typed one level deep, FAILURES names the new codes; health schema 6,576 chars). Builder opus for 41043ad (dispatched before the routing correction), sonnet for the follow-ups. Cut
  INSTRUCTIONS to about half: paging text once, judgment disclaimer once, no
  motivational framing; list all eight evidence kinds in the `query`
  description; shrink `health`'s schema to what a model acts on or mark it
  operator-only; SQL errors carry `data.code` and `remedy`; an unknown
  `structure` language says so; fix `examples/queries/README.md` `LIKE` on
  payload and split `blast_radius.sql` into single-statement recipes; make the
  first `report` after `project_control on` not fail with `stale_revision`
  (sync once inside `on`, or let the following call adopt the new revision).
  DoD: `check_mcp_surface --record`; the deep-dive stdio script
  (`on` → `report`) succeeds first time.
- **C3 — release readiness.** State: **landed** — `47a4924` (changelog + 1.1.0) + `152508b` (release profile; 18,693,064 -> 12,380,424 bytes; `check_performance` PASS) + `f43dc9b` (Luna: changelog corrected against what 1.0.10 actually shipped, Removed and Known-issues sections). `--version` prints `slugaudit 1.1.0`. Tagging and pushing remain the owner's call. Builder sonnet. `CHANGELOG.md` entries
  for everything since 1.0.10; version `1.1.0`; README install paths agree
  with the binary's `--help`; `[profile.release]` with `strip = true`;
  `check_version_consistency` passes. Tagging and pushing are the owner's
  call and are not part of this phase.

- **C4 — concurrent fresh open (architect 2026-10-26).** State: **landed** — `ef3b98c` (cherry-pick of `fb5a88b`): busy_timeout before `journal_mode=WAL`, a bounded WAL retry, IMMEDIATE schema init re-checking `user_version`, and a locked re-check before discarding on the open-failure path; paired trials 8/20 failing -> 0/20 and 0/40; 4 processes 0/10. **Still open:** the stale-row discard (`sync/manager_orchestration.rs` `verify_metadata_or_rebuild`) and `project_control`'s recovery still unlink without the lock (narrow two-session window after an upgrade). Builder sonnet.
- **C5 — mechanic scan cache.** State: **landed** — `e13e0cf` + `5dee1ae` (one retention budget per pass; cache tests moved out; key tests for pattern/project/edited entry). 2,000-file Python project: cached `leads` 0.065 s vs 0.156 s. Builder sonnet.
- **C6 — sweep.** State: **landed** — `d6795df`: stale `config.json.*.tmp` removed on load (only when `config.lock` is free), every overridable limit refuses 0 with one warning, one shared `parse::budget::parse_within`, DEVIATIONS header now says oldest-first, B0 warn-once test race fixed. Edit list drafted by sonnet, sections 1–2 applied by a haiku junior (3.4 h, stopped correctly on the race), finished by sonnet.

- **C7 — output-schema size (architect spot-check, 2026-10-26).** State: **still
  open**. After Wave C the `tools/list` wire content is 104,818 chars (was
  67,277 before Phase 8): output schemas are 82k of it (`report` 19.1k,
  `mechanic` 16.6k, `leads` 12.8k, `query` 8.4k, `structure` 8.2k,
  `finding_read` 6.6k, `health` 6.6k), with 414 `description` keys and the
  shared `Page` def embedded once per tool. Descriptions plus input schemas are
  20.9k chars (~5.2k tokens) and INSTRUCTIONS 5,485, so a client that forwards
  only those pays about what it did before Phase 8 with one more tool; a client
  that forwards `outputSchema` pays ~26k tokens per session. Target: type
  nested response objects one level deep as `health` now does, drop
  per-property descriptions that repeat the field name, and bring every output
  schema under 4k chars. Sonnet builder, Luna review of the surface diff,
  `check_mcp_surface --record`.

### Recorded, not scheduled (D9)

- **Global config.** State: **not scheduled**. A global config at `~/.slugthug/slugaudit/config.json`. Precedence: built-in < global < per-repo `config.json` < `SLUGAUDIT_*` env; each layer overrides only the keys it sets. Includes a global `mechanics` section, which adds a fourth source value for where a mechanic came from.
- **Per-repo mechanics file.** State: **not scheduled**. Possibly split per-repo mechanics out of `config.json` into their own file.
- **Deferred wave-1 evidence findings.** State: **not scheduled**; evidence in `.planning/archive/remediation_2026-10-28_wave1.md` (Evidence table; scripts `scratchpad/evidence/E1..E5`):
  - H1, healthy-watcher tree walk per call (E1): `health` ~0.1 ms without a path and 2, 6, 52 ms at 5k/10k/100k with a path; `query`/`finding_read` with no changes 0.5-2 s p50 (748/713/1345 ms at 5k/10k/100k), weak scaling; cause unprofiled, inconclusive.
  - M2, CAS race under concurrency (E2): 0 Busy in 100 attempts, but 95 of 520 calls failed with JSON-RPC -32603 "concurrent publish won the race ... retry the call", up to 8.6 s.
  - M4, stdout stall (E3): pipe full at 2 s, completions stuck at 74 of 1476, RSS +0.5 MB/s; recovered after the reader resumed. The run only hit the panic error path (`query` with a progressToken panics at `src/server_runner.rs:311`, a separate defect from the report). State: **resolved**, fixed by `17835d9` (progress notifications spawn on the captured runtime handle), reproduced on `4a70544` and confirmed gone (4 stdio variants) on the fix.
  - M5, stdout stall (E4): stalls at call 143 (`health`, ~437 B/call) with the default filter, or call 381 (unknown tool); debug stalls during startup; draining releases it. Confirmed.
  - Related (E5, M12): `config.json` keys are re-sorted alphabetically on rewrite with no trailing newline (observed before P8); the "order preserved" claim was wrong.
- **Stored-content cap review.** State: **not scheduled** (architect 2026-10-25: a Wave C profile decision). `query`'s `SQLITE_LIMIT_LENGTH` is max(`max_query_value_bytes`, 3 x `max_file_bytes`), so the zeroblob/hex/replace amplifier is bounded but can reach ~1.5 GB (estimated) on the audit profile; the real fix is a lower default for stored content.
- **Wave C residue.** State: **still open**, small: the stale-row discard (`verify_metadata_or_rebuild`) and `project_control` recovery still unlink without C4's lock; a `user_version` 0 database corrupt past page 1 is now reopened rather than discarded (reasoned, untested); the warn-once ledger is thread-local under `cfg(test)` (`f973bc6`) because `tools::mechanic::persist_tests` builds limits from env without `TEST_ENV_LOCK` — that test should take the lock; the language-pack alias table can drift in one direction.
- **Wave B residue.** State: **still open**, small: two servers opening a fresh project database at once can fail one with `database is locked` at connection configure (2 of 7 trials; fits the per-project sync lock above); the parse-under-deadline loop exists three times (`structure_exec::parse_within`, `evidence/calls/query_calls.rs`, the pack-timeout path); `config.json.<pid>.<n>.tmp` files left by a crash are never swept; a custom mechanic re-parses its language's files on every `leads` call that selects it; the alias table can still drift in the pack-adds-an-alias direction.
- **`rem2/LG` (real database/watcher paths in recovery and notify WARNs).** State: **still open**, left out of 1.1.0 after two failed reviews. Branch `rem2/LG` (commits `e2cafbc` + `9071645`) is complete except `src/tools/health.rs:283,294,305`, which still logs `database_id = redacted_path(..)` (3-line fix plus a capture test); everything else in it reviewed OK.
- **CLI trailing arguments.** State: **not scheduled**. `install` and `menu` now reject extra arguments (`17835d9`'s sibling `83b32de`); `serve`, `help`, `update`, `version` and `--help` still silently ignore trailing arguments (`src/cli.rs` ~63-73).
- **`ResourceLimits::discover()`.** State: **not scheduled**. At `src/sync/discovery.rs` ~268 it uses `ResourceLimits::default()` and ignores `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`; only tests call it. Fix: drop `pub` or take a limits argument.
- **Tool-level limits-error test.** State: **not scheduled**. No test checks that a tool call returns the internal error when limits are uninstalled and the env is invalid (the `map_process_limits_error` / `process_limits_for_tool` wiring is untested).
- **`project_control off` on a corrupt `project.db`.** State: **still open**, needs an owner decision: `off` fails because `activation_lock` opens the database to lock it. Should `off` unlink an unopenable database?
- **Rebuild-path residual window.** State: **still open**. An `off` landing after the sync path's own discard and before `publish_from_scratch`'s creating open can recreate `project.db` (documented in ARCHITECTURE.md, activation section). The fix needs one lock across discard plus create.
- **`config.lock` unlinked by `off`.** State: **not scheduled**. `off` unlinks `config.lock` while another process may hold its flock (documented).

- Watcher removal in favour of the sweep plus a re-hash of files whose stored
  mtime is within 2 s of now, behind a per-project in-process sync lock.
  State: **not scheduled** — design agreed in principle 2026-10-25; a large
  refactor that waits for Waves A–C.
- `touch_unchanged` writes outside `publish_revision`; `sync` returning
  `rmcp::ErrorData`; `ignore_rules/build.rs` `.ok()` fail-open; `netfs.rs`
  `unwrap_or("")`; "dropped" vs deferred events doc drift; FIFO swap race in
  `capped_read.rs`. State: **still open**, small, suitable for a junior sweep
  after Wave C.
- Instrumenting the founding metric (call chains per session) is not in the
  code; the server log already carries `session_id`/`call_id` per call, so a
  log-derived measure is possible without a code change. State: **not scheduled**.

## Phase 9 — Immutable paged results (the result store)

State: **not started** (approved by the owner 2026-10-26; architect's design
and evidence in `.planning/archive/design_2026-10-26_result_store.md`, which a
builder reads before touching paging). Branch `fix/deep-dive-2026-10-25`, one
commit per item, never push. Phase 9 is complete only when every acceptance
check in 9.7 passes on the integrated tree; avoidable defects are fixed, not
recorded as limitations.

Contract (owner): new requests sync and describe the current tree; a paged
result is one immutable answer and every page describes the revision page one
was taken from; edits while reading never alter the result being read; no
silent skip, repeat or revision mix; exhaustion and expiry are explicit; the
agent copies one cursor and manages no revisions; the grace period never
invalidates a report, it only governs reclaim eligibility under pressure, and
continuing a result renews its protection.

- **9.0 — Golden capture.** State: **landed** e85bb10 (22 fixtures, `tests/golden/phase9/generate.sh`, 14 passing tests; NaN is unreachable in the bundled build even through a `cfg(test)` function, SQLite stores it as NULL, so the NaN fixture records NULL cells and an empty `non_finite_cells`). Before any paging change,
  capture per-tool response fixtures from `2ad7bc3` on the existing test
  fixtures (`leads` counts and rows incl. compact, `query` incl. clipped,
  non-finite and NUL-text cases, `structure` single-file and project-wide,
  `finding_read`, `mechanic` add/list) into `tests/golden/phase9/`. DoD: the
  fixtures are committed with the script that produced them; a test loads
  them (it is allowed to fail until 9.3 lands and is marked so).
- **9.1 — `store/result_store/`.** State: **landed** cdfc0ac, review fixes 9da1f03, test hardening 194cea4 (Victor and Quinn reviewed; non-unix stub not compiled, no Windows target installed; `max_result_store_bytes` is derived from the profile's response ceiling and does not follow an env override of `max_query_response_bytes`). File lifecycle
  (create under `.planning/slugaudit/results/`, open, unlink, pragmas), owner
  lock file, startup sweep, schema (`meta`, `rows` with untyped columns and the
  `nonfinite` marker), the `Materialize` trait, batched writer (1,000 rows per
  batch, meta last, registry after commit, rollback-and-remove on any error),
  ordinal page reader with the existing byte cut, registry with
  active/idle/delivered classes, admission (fit → admit; else reclaim
  delivered oldest-first, then idle oldest-first, never active; else refuse),
  cursor codec (`v1.<nonce>.<result>.<ord>`), stats, limits
  `max_result_store_bytes` (4 × `max_query_response_bytes`) and
  `result_active_grace_secs` (900) with env/profile/config entries documented
  in `limits.rs`. Platform: Unix unlink-after-open; any other target fails
  closed at open with `result_store_unavailable`. DoD (unit): fidelity matrix
  round-trips every storage class and records NaN/±Inf in `nonfinite`; same
  token twice → identical rows; disjoint pages cover the result exactly once;
  admission order as specified and an active result is never reclaimed; an
  idle result older than the grace period is still readable and becomes
  active on continuation; a result over budget leaves no file; a failure
  mid-write leaves no file and no registry entry; full-disk seam →
  `result_store_unavailable`; abort mid-write leaves no file (process-spawn
  test); a planted leftover is swept at startup while a locked one is not;
  cross-thread insert/read through the per-result mutex.
- **9.2 — Shared pagination layer.** State: **landed** d3119fc, wrong-tool precheck d5c7694. `Page` keeps `next_offset` until the last 9.3 migration removes it; `check_mcp_surface` fails on 25 `Page`/`Completeness` keys, expected until 9.3 records it. Follow-up for 9.5: a wrong-tool cursor does not increment the `unknown_cursors` stat. `tools/context.rs`
  gains one `paginate` entry: without `cursor` it syncs, opens the verified
  read transaction, materialises via the tool's `Materialize` producer and
  serves page one; with `cursor` it serves the stored page with no sync and no
  revision check. `tools/page.rs` gains `cursor`, `completeness`; loses
  `next_offset`. Error codes `unknown_cursor`, `result_expired`,
  `result_store_full`, `result_too_large`, `result_store_unavailable` with
  remedies in `error_codes.rs`. DoD: unit tests for both branches and every
  code; `check_source_limits` passes with a cohesive module, no artificial seam.
- **9.3 — Tool migrations** (one builder per tool, disjoint files, after 9.2).
  State: **not started**. `leads`: rows and `mechanic_counts` from one
  transaction, `e.id` tie-breaker, custom scans may be `partial`. `query`:
  all-or-nothing, remove the separate `count(*)`, `total_rows_unavailable`
  and its budget; `total` exact. `structure`: partial semantics on the stored
  result (`complete_scan`, `limited_by`, `resume_after_file`), resume is a
  new request. `finding_read` and `mechanic`: cursor paging. `offset` and
  `expected_revision` removed from all five. DoD per tool: golden equality
  with 9.0 fixtures apart from the paging fields; the edit-between-pages test
  (page one, edit so one row vanishes and one appears, read all pages, union
  equals a full fetch taken at page one, `total` unchanged, then a fresh
  request shows the edit).
- **9.4 — Exclusion and database-replacement proofs.** State: **not started**.
  Tests that `results/*.db` and the lock file are excluded from discovery, the
  stat sweep, the watcher scope and reconcile (create them, run a full publish
  and a sweep, assert no `files` row and no watcher event reaches reconcile);
  tests that each of the four database-replacement paths (stale root,
  incompatible version, corruption recovery, `project_control off`) runs while
  a result is open and the continuation still completes; a publish and
  `wal_checkpoint(TRUNCATE)` succeed with results open and the WAL truncates
  to zero. DoD: all pass; `ARCHITECTURE.md` gains the invariants (immutable
  paged result; continuation does not sync; nothing addressable before
  commit; active results never reclaimed; result store and `project.db` hold
  disjoint data).
- **9.5 — Surface, docs, authorizer.** State: **not started**. INSTRUCTIONS
  paging paragraph rewritten around `cursor`; store stats in `health` detail;
  the read-only authorizer denies `CREATE TEMP TABLE`; `docs/TROUBLESHOOTING.md`
  gains the result-store section (location, cleanup, platform policy, the
  five codes); CHANGELOG under 1.1.0 (breaking: `offset`/`expected_revision`
  removed, `cursor` added); `check_mcp_surface --record`; Luna reviews the
  surface diff and the INSTRUCTIONS text. DoD: all gates; INSTRUCTIONS size
  reported in the commit body.
- **9.6 — Budget confirmation.** State: **not started**. On the 68k-line
  project copy under `audit`, measure: materialisation time and file size for
  `leads` all rows, `query` `SELECT path, content FROM files`, project-wide
  `structure`; peak RSS with eight results open; confirm or adjust the
  multiplier 4 and grace 900 s, recording the numbers in the design record.
- **9.7 — Acceptance (phase gate).** State: **not started**. Full README gate
  list via clerk; the stdio MCP end-to-end against a fresh release build via
  quinn: `on` → `leads` page one → edit a file removing one match and adding
  one → `leads` with cursor until `complete` (union equals a full fetch taken
  at page one; totals unchanged) → `leads` without cursor shows the edit →
  same cursor twice gives identical pages → final-page retry works → a token
  from a restarted server gives `unknown_cursor` → `query` with NaN (via the
  test-registered function if no SQL expression yields NaN in the bundled
  build), +Inf, NUL text, blob and numeric-looking text pages identically to
  the 9.0 golden → `structure` project-wide on a window-limited project
  reports `complete_scan: false` and resumes as a new request → `health`
  verbose shows store stats. Phase 9 is **landed** only when 9.0–9.7 are all
  landed with commit hashes here.

Recorded, not scheduled, from Phase 9: fold the mechanic scan cache into the
result store (same mechanism keyed by revision); a `release` action for
abandoned results if admission refusals are ever observed in practice.

## Remaining P1 debt (historical; superseded by wave 5)

State: **superseded** — the wave-5 audit is a fresh, complete inventory
(282/282 files) and its verified ledger is the current defect record. All 56
confirmed P0/P1 entries it found were fixed; the dispositions below are kept
as history so a later reader can see how the strict-audit debt closed. No item
below is carried forward as a confirmed P0/P1 defect.

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
  2 closed the boundary/unreachable-arm subset (`ed58a4b`). The wave-5
  verification audit re-derived the inventory from the current tree and
  confirmed 11 R5 entries (5 test-only), **all fixed** in wave 5. The older
  142-entry list is historical; the final ledger is the current record. The
  two accepted finding arms under *Accepted arms and discarded variants*
  remain accepted, not open.

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
- Watch registration is **non-recursive**: the scope walk enumerates every
  indexable directory and each is watched individually, so kernel watches are
  bounded by the scope caps. A directory created later is registered on the
  next scope refresh; the per-call stat sweep covers the gap
  (`src/watch/manager/scopes.rs`, `src/watch/scope_apply.rs`, 2026-10-19).
- Ignore rules are **project-scoped** (`parents(false)`): ancestor ignore
  files above the project root are not applied, so full publish, the watcher,
  the sweep, and reconcile share one rule set (`src/ignore_rules.rs`).
- `ImportReport.skipped` is a **count**, not an unbounded list; the
  per-reason detail is `report.completeness` (`src/tools/project_control_types.rs`).
- The connection `Configure` arms and the one-shot security rejections in
  `connection_guards.rs` remain **accepted as documented** (unreachable
  without an invalid handle/OOM; raw paths only under the 2026-10-16 one-shot
  carve-out).
- Tests are exempt from the production panic ban: a panic is the failure
  signal there.
- The 2026-10-19 strict audit counts test/bench/fuzz R1/R2 entries, but the
  crate's `cfg(test)` exemption is intentional; those counts are not a defect
  list to burn down mechanically.

## Documentation policy

State: **landed (wave 2; records extended through wave 5)** — authored at
`7a782de`, with the AGENTS.md parser-pack path corrected at `326ec9a`; the
durable audit evidence under `.planning/archive/` now spans eight
`audit_2026-10-19_*` files, the last two added by wave 5.

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
set. The eight `audit_2026-10-19_*` files in `.planning/archive/` are
historical evidence copies, not canonical docs, and are not read by the gate.
