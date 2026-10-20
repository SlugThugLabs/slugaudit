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

The two audit reports this plan refers to are session artifacts outside the
repository:

- `/tmp/audit2/AUDIT_REPORT.md` — the 2026-10-19 strict audit of `main` at
  `26ecdb4`: 259/259 `.rs` files (50,325 lines), 2,550 confirmed violations.
- `/tmp/audit3/WAVE_REPORT.md` — the wave-1 integrated report: commit map,
  findings fixed, public-schema changes, accepted unreachable arms, and the
  final gate at `fe923a7`.

Anything from those reports that must survive belongs in this file, because
`/tmp` is not durable.

## Current state (2026-10-19)

- `main` at `6241b43` (`docs: remove the superseded planning records`).
  Wave 1 landed at `fe923a7`; the only later commit is the deliberate deletion
  of the superseded planning record.
- Crate `slugaudit-mcp-rust` `1.0.10`, edition 2024, `publish = false`,
  licensed by `LICENSE` (PolyForm Noncommercial 1.0.0 plus the SlugAudit
  Tool-Use Additional Permission).
- Tests: **1,156 passed, 0 failed, 1 ignored** at `fe923a7` (1,030 before
  wave 1; wave 1 added 126).
- Line coverage: **92.85%** (11,212 / 12,075), gate 83%; the strict audit
  measured 90.26% before wave 1.
- `check_docs_drift` and `check_mcp_surface` fail in the wave-1 tree only
  because their inputs were deliberately deleted. Wave 2 restores both as
  **freshly authored** artifacts; nothing is restored from git history.

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
  `/tmp/audit3/WAVE_REPORT.md`.
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

Still open from wave 1:

- The unreachable/untestable arms listed under *NEEDS-DECISION* in
  `/tmp/audit3/WAVE_REPORT.md` (install/update hooks, `finding` line-count
  conversion, read-transaction start, a discard path with no seam, the netfs
  positive rejection, `Configure` arms, `project_control` serialize/discard/
  reopen arms, `connect` binary path and stdout-flush arms).
- `report` counted non-UTF-8 paths with a bounded discovery walk on every
  call. Truthful, but I/O on a read path; wave 2 persists the count at sync.
- `check_docs_drift` and `check_mcp_surface` failed because their inputs had
  been deleted, not because of code.

## Phase 4 — Wave 2 (in progress)

Each item was commissioned on 2026-10-19 and is **in progress** until the lead
runs the full gate and closes it.

| Item | Package | State | Acceptance |
|---|---|---|---|
| Canonical docs authored fresh (this file, `archive/DECISIONS.md`, `docs/CLIENTS.md`, `docs/TROUBLESHOOTING.md`, `docs/RELEASING.md`, `docs/about.toml`, `docs/third-party-notices.hbs`) | wave2-docs | in progress | `cargo run --quiet --bin check_docs_drift --locked` PASS; no deleted doc restored from git |
| MCP surface snapshot recorded from the live surface | wave2-surface | in progress | `check_mcp_surface --locked` PASS against a newly recorded `.planning/archive/mcp_surface.json` |
| Persist the non-UTF-8 path count at sync; `report` reads it from the database instead of walking the filesystem | wave2-report | in progress | report responses keep `unrepresentable_path_count` / `index_partial`; no per-call discovery walk |
| CLI writes fail gracefully on a closed pipe (`BrokenPipe` is a clean exit; no panic) | wave2-cli | in progress | closed-pipe test; write errors propagated or logged once |
| Remaining unreachable failure arms tested through a clean refactor, or explicitly accepted/removed; no production-only test hooks, no lint suppressions | wave2-failure | in progress | every arm has a test, a documented acceptance, or is removed |
| Fresh performance baseline recorded after the code changes | lead, after integration | in progress | `check_performance -- --record` against the integrated tree; baseline carries its machine |

## Remaining P1 debt (still open)

These are production entries from `/tmp/audit2/AUDIT_REPORT.md` that wave 1
did not close. They are recorded here so a later wave does not rediscover
them from scratch.

- **R4 — functions/files over size targets: 109 entries, 40 production**
  (45 functions > 50 lines, 54 files > 200 code lines, 10 functions with
  cyclomatic complexity > 15). 18 production files are above 200 code lines;
  several carry `slugaudit-line-exception` annotations because the project
  gate permits 200–300 with approval. **Still open.** Splitting them is a
  reviewable refactor, not a mechanical one.
- **R8 — public docs / external use: 258 production entries.** The rustc
  `missing_docs` subset is 239 warnings at 212 unique sites; the audit count
  also includes public items with no external reference (dead public API).
  **Still open.** Do not claim this is fixed.
- **R5 — failure-path tests: 142 entries (110 production, 32 dev-gate).**
  Wave 1 targeted 107 in-scope entries and added 126 tests. The dev-gate/
  test-side remainder and the accepted unreachable arms above are
  **still open** unless individually resolved by wave 2's failure-arm package.
- **Report walk on the read path** — superseded by wave 2's report-count item;
  keep it listed until that item lands.

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
- Mutation testing covers exactly `sync/revision.rs`, `sync/publish.rs`,
  `sync/hash.rs`, and `tools/context.rs`; widening is a CI-minute decision.
- Tests are exempt from the production panic ban: a panic is the failure
  signal there.
- The 2026-10-19 strict audit counts test/bench/fuzz R1/R2 entries, but the
  crate's `cfg(test)` exemption is intentional; those counts are not a defect
  list to burn down mechanically.

## Documentation policy

State: **in progress** — the documents are authored in this wave; the lead
closes the item after the full gate and integration.

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
document is restored or copied from git history. `check_docs_drift` enforces
existence, the module-map paths, and the `*_tests.rs` count claim. The release
docs (`docs/RELEASING.md`, `docs/about.toml`,
`docs/third-party-notices.hbs`) are consumed by
`.github/workflows/release.yml` and are not part of the drift gate's checked
set.
