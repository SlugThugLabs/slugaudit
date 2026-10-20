# Production-hardening wave — integrated report

Repo: `/home/slugaudit`, `main` at `fe923a7` (base `26ecdb4`). Date: 2026-10-19.
Work was split into ten isolated worktrees, then cherry-picked onto `main`.
Three integration fixes followed: the publish-log category arm for the new
`PublishError` variant, batching for stale-finding invalidation, and a
test-only clippy initializer fix.

## Commit map

| Package | Agent commit | On `main` | Subject |
|---|---|---|---|
| a1 sync import | `2855bef` | `0fdf1f4` | harden sample bounds and manifest batching |
| a2 sync publish | `7dbb11d` | `371e08a` | redact publish logs and remove duplicate failure logs |
| a3 sync manager | `fda57ad` | `45c18bb` | reconcile concurrent revision changes and cover failure paths |
| a4 watch/store | `b8c8917` | `d1888ee` | redact watcher errors and bound SQL text functions |
| a5 tools finding | `f1b5be2` | `f3f7cb5` | page finding reads and type database errors |
| a6 tools structure/query | `cfc6f0c` | `e615bd1` | bound structure/query inputs and make scan completeness truthful |
| a7 tools surfaces | `8be441b` | `597c734` | make report, health and project messages truthful |
| a8 server/resources/evidence | `c5e6fab` | `cd88495` | sanitize failure logs and share evidence span construction |
| a9 cli/connect | `57050d9` | `cce4285` | initialize logging before dispatch and make connect messages truthful |
| a10 update/install | `154914e` | `7c194e2` | assert exact failure variants |
| integration | — | `4b46625` | classify the sampling index error in publish logs |
| integration | — | `c1c6628` | batch stale-finding invalidation paths |
| integration | — | `fe923a7` | use struct update in the scope unwatch test |

## Findings fixed

The 55 confirmed production findings from the strict audit:

| Rule | Count | What changed |
|---|---:|---|
| R1 | 3 | `sample_batch` indexing is checked; out-of-range fails typed, never panics |
| R2 | 1 | a later distinct sampling error is logged once instead of discarded |
| R6 | 22 | report/health/count/phase/message truthfulness; DB read failures no longer blame caller input; non-UTF-8 discovery is counted |
| R7 | 5 | manifest path-set paging, finding-read byte paging, structure field caps, SQL text-function cap, resource-URI cap |
| R10 | 3 | shared evidence span builder; shared session-purge helper |
| R12 | 21 | raw paths/SQL/env values removed from correlation logs; duplicate failure logs reduced to one; watcher/notify/ignore errors no longer format paths |

Plus:

- Top-10 `manager_health.rs:299`: the CAS baseline is re-read every barrier
  iteration, so a change during iteration 1 is reconciled in the same call.
- Residual R7 found during integration: `revision.rs::invalidate_stale_findings`
  now batches touched paths (4096) instead of one unbounded JSON parameter.
- 107 in-scope R5 failure-path entries were targeted; 126 tests were added in
  total (1,030 baseline → 1,156), with the unreachable arms listed below. The
  three dead variants (`SyncError::Revision`, `SyncError::Read`,
  `DiscoveryError::Walk`) are reported, not tested.

## Public API / schema changes

- `PublishError::SamplingIndexOutOfRange { index, total }` (documented).
- `finding_read` request `offset`; response `findings_bytes`, `page_bytes`,
  `next_offset`; entry `detail_clipped` (documented).
- `ReportResponse.completeness.unrepresentable_path_count`.
- `HealthPhase::Unknown`; the health tool description no longer promises
  "most-recently-touched" selection.
- `ActivationError::DatabaseLock` replaced by `DatabaseOperation`; BUSY/LOCKED
  remains `DatabaseBusy`.
- `ConnectError::AddFailedNoOutput`; `DisconnectOutcome`; `disconnect_agent`
  returns it.
- Schema descriptions changed for `QueryRequest::sql`, `StructureResponse::
  files_not_scanned`, `PageLimit::Bytes`, `files_covered`, and the
  unhandled-panics resource recipe.
- The MCP surface snapshot was not re-recorded: `.planning/archive/mcp_surface.json`
  is deliberately absent and was not restored.

## NEEDS-DECISION (unreachable/untestable without a production hook)

- `install.rs` `CurrentExe`, `Clock`; `update.rs` `UnsupportedPlatform` is
  compile-gated to non-Linux-x86_64.
- `finding.rs:243` u32 line-count conversion (unreachable for storable content);
  `finding_read.rs:129` `query_map` bind failure (unreachable).
- `context_transactions.rs:14` read-transaction start failure (fresh connection
  cannot fail `BEGIN DEFERRED` without an active transaction).
- `manager_orchestration.rs:132` discard failure on the stale-metadata path (no
  seam); dead sync variants as above.
- `store/netfs.rs:70` positive network-filesystem rejection (no hermetic network
  mount); `store/connection.rs:324` Configure error arms (unreachable; the 5 s
  timeout behavior is tested).
- `project_control.rs:178,186,188` serialize/discard/reopen failure arms
  (unreachable without a hook or non-root FS).
- `connect.rs` `BinaryPath`; `stdout().flush()` arms (the preceding `print!`/
  `println!` panics first on a closed pipe — a separate latent CLI-panic design
  item).

## Residual notes

- `report` now performs a bounded discovery walk on every call to count
  non-UTF-8 paths (bounded by `max_sync_wall_clock`). It is truthful, but it is
  I/O work on a read path; persisting the count during sync is the follow-up if
  report latency matters.
- `revision.rs` is at 198 code lines (one line below the 200 exception floor).
- The docs-drift and MCP-surface gates cannot pass while the deleted docs and
  snapshot remain absent. They were not restored, per instruction.
- `check_performance` is uncheckable against the deleted baseline; the report
  walk is the main unmeasured performance risk.

## Final gate (clean `target-final`, `main` at `fe923a7`)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass |
| `cargo test --lib --bins --tests --all-features --locked --no-fail-fast` | **1,156 passed, 0 failed, 1 ignored** |
| `cargo deny check` | pass (advisories/bans/licenses/sources; one unmatched license exception warning) |
| `cargo machete` | pass, 0 unused |
| `check_coverage` | **92.85%** line coverage (11,212 / 12,075; gate 83%) |
| `check_source_limits` | pass |
| `check_no_duplicates` | pass |
| `check_producer_coverage` | pass (8/8 EvidenceKind, 4/4 ExtractionCompleteness) |
| `check_governance` | pass |
| `check_version_consistency` | pass |
| `check_docs_drift` | **fail** — `.planning/PLAN.md`, `.planning/archive/DECISIONS.md`, `docs/CLIENTS.md`, `docs/TROUBLESHOOTING.md` are deliberately deleted |
| `check_mcp_surface` | **fail** — `.planning/archive/mcp_surface.json` is deliberately deleted |

The first gate run failed only on a test-file clippy lint; `fe923a7` fixed the
initializer and the gate was re-run from the final tree. The two failures are
the deleted-file environment, not code: no deleted doc or snapshot was
restored. `check_performance` was not run because its baseline is deleted.

Worktrees, agent branches, and `target-final/` were removed after the gate.
`main` is local at `fe923a7` and was not pushed. The working tree still has the
16 deliberate doc deletions, uncommitted.
