# Wave 5 verification audit

Repository: `/home/slugaudit`, `main` at `66536c0` (code at `4b765e1`; the
last commits after it are docs only). Audit date: 2026-10-19.
**Files reviewed: 282 of 282 (57,449 lines)** by 12 independent chunk auditors,
plus a verification pass on every P0/P1 entry.

## Detectors (current tree)

| Detector | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass |
| `cargo test --lib --bins --tests --all-features --locked --no-fail-fast` | **1,246 passed, 0 failed, 1 ignored** (post-fix final gate) |
| `cargo deny check` | pass (one unmatched license-exception warning) |
| `cargo machete` | pass |
| `check_coverage` | **93.56%** line coverage (12,005 / 12,831; gate 83%) (post-fix final gate) |
| `check_source_limits` | pass; **0 production files >200 code lines** |
| `RUSTFLAGS='-W missing_docs' cargo check --lib --bins` | **0 warnings** |
| `cargo doc --no-deps --lib` | **0 warnings** |
| `cargo crap --min 0 --path src` | **0 production functions with CC >15** |
| clippy `too_many_lines` threshold 50 | 0 production warnings; 1 test-only inline warning |
| `check_no_duplicates` | pass |
| `check_producer_coverage` | pass |
| `check_governance` | pass |
| `check_version_consistency` | pass |
| `check_docs_drift` | pass |
| `check_mcp_surface` | pass |
| `check_performance` | pass (0.97x–1.09x, threshold 20%) |

## Post-fix final validation

The final tree after all 56 fixes (`2a65a85` plus the integration fix) passes the
complete gate: fmt, clippy `-D warnings`, 1,246 tests (0 failed, 1 ignored),
deny, machete, coverage 93.56% (12,005/12,831), source limits (0 production
files >200; `manager_orchestration.rs` 197), duplicate checks, producer
coverage, governance, version consistency, docs drift, MCP surface, and
performance (0.99x–1.02x against the 20% threshold).

## Manual audit

The 12 chunk ledgers are in `/tmp/audit7/chunk1.md` … `chunk12.md`; the
verification pass is `/tmp/audit7/verified.md`.

- Author candidates: 65 P0/P1 rows (17 P0 / 48 P1).
- Verification: **56 confirmed (12 P0 / 44 P1)**; 9 duplicate rows merged into
  confirmed rows; **0 dropped as false or already covered**; 9 suspected
  misses recorded separately.
- Scope: 49 production, 2 dev-gate, 5 test.

### Fixes

Every confirmed P0/P1 was fixed on `main` by the wave-5 fix set (the commits
after `4b765e1` and the integration fix `730699f`):

- **P0:** fail-closed sweep/discovery on unreadable trees and malformed ignore
  files; duplicate top-level SQL columns refused; notify-callback deadlock
  removed; NFS-root detection; false disconnect success and 0600/symlink client
  config writes; cross-session finding purge moved to session start; path
  redaction in ignore-rule, sweep, manager open/recovery, reconcile/drain, and
  parser-load logs; finding-read offset test.
- **P1:** health/structure error typing and unreadable-DB phase; report
  operation context; project-control ImportReport fields and
  modified/deleted/skipped; gitignore sync/FIFO/symlink durability; query-clip
  continuation and count wall-clock test; sweep function decomposition and
  discovery `entry.error()`; ignore parent/hidden consistency; watch scope
  bound, dead state APIs, real poison test, util lock labels, deterministic
  event tests; model/graph dead API removal and Python relative imports;
  update `run_update` shipped-platform tests; docs-gate parser and source-limit
  approval bypass; test fixture leak, finding caps, redaction control lines,
  connect I/O/stat assertions.

### Accepted decisions / residual P2s

- Watch scope is now `NonRecursive`; a newly created nested directory is
  registered on the next scope refresh, and files created inside it before
  that are caught by the per-call sweep (documented `Partial` model).
- Ignore rules are **project-scoped** (`parents(false)`); ancestor ignore files
  outside the project root are no longer honored. This matches the module's
  documented rule set.
- `ImportReport.skipped` is a count, not an unbounded list; details remain in
  `report.completeness`.
- `connect_cli` argv still uses a lossy binary path for CLI-strategy clients;
  `write_atomic` does not fsync before rename. Both are P2 follow-ups.
- The ignore-error mapper in `ignore_rules/build.rs` duplicates the watch
  helper; deduplication is a P2 follow-up.
- Documented one-shot security rejections in `connection_guards.rs` keep raw
  paths under the 2026-10-16 carve-out; connection `Configure` arms remain
  accepted as unreachable without an invalid handle/OOM.

Detector-only rules (R4/R9/R11) are the table above; test-only
R1/R2/R4/R10 are counted, not enumerated, per the worker brief.

## Uncheckable / not run

- `cargo-mutants` is CI-only; the local scope is six files and was updated in
  wave 4. Not run here.
- The fuzz targets need nightly + sanitizer; not run.
- The 10k-file soak is `#[ignore]`d; not run. Its 64-file companion ran in the
  full suite.
- Coverage is line coverage, not branch coverage.
- The release workflow's reproducibility/attestation jobs need GitHub Actions;
  the cargo-about notices render was validated locally.
