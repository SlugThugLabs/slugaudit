# Task Plan Status

## 2026-10-19 — Production resource-bounding remediation

Branch: `fix/resource-bounds-2026-10-19`, created from `0f568d2` (pushed `origin/main`;
that commit is the recovery checkpoint). Source: the 2026-10-19 algorithm-scale follow-up
(R7 items 1–4) plus the documented-maxima investigation.

Do not mark a row resolved until its implementation and regression tests exist and pass.

| # | Finding | State | Evidence |
|---|---|---|---|
| 1 | `src/project/activation.rs:248` — `.gitignore` read with `std::fs::read`, no cap | **resolved** (`dd87904`) | 1 MiB `MAX_FOREIGN_TEXT_BYTES` cap through `read_file_capped_bytes`; over-cap/unreadable → `NotWritten` with the observed error, file byte-identical, project still enabled; missing file still creates the rule. `cargo test --lib project::activation`: 30 passed, 0 failed. |
| 2 | `src/sync/hash.rs:21` — whole-file hash read has no cap; stat/read race | **resolved** (`94726ab`) | `hash_file` streams `take(max + 1)` in 64 KiB chunks into BLAKE3 and refuses before feeding a byte past the cap; `TooLarge` becomes a skipped existence row. `cargo test --lib sync::hash`: 11 passed, 0 failed. |
| 2a | `src/sync/sample.rs` — sampling has the same unbounded read after its stat | **resolved** (`94726ab`) | `sample_file` uses `read_file_capped_bytes`; `TooLarge` carries the observed length. `cargo test --lib sync::sample`: 19 passed, 0 failed. |
| 3 | `src/watch/scope.rs:39` — unbounded watch-scope walk under the manager lock | **resolved** (`7d87f04`, `a1147f9`) | Walk bounded by `max_watch_entries`/`max_watch_depth` (Lean 50k/32; Audit and Adaptive 200k/64; env overrides); walk and `IgnoreRules` build outside the manager lock; a scope-refresh generation preserves an ignore-file event arriving mid-walk; `Partial` health / `degraded` phase; full verification refreshes the scope before draining. Tests: watch 57, limits 11, profile 8, `sync::manager` 37, `tools::health` 31; MCP surface re-recorded and PASS; 10k soak passed. |
| 4 | `src/sync/reconcile/pipeline.rs:183` — whole dirty-path set serialized before the first deadline check | **resolved** (`94726ab`) | 4096-path batches in `query_existing_hashes`; deadline checked before every batch and the final flush; a spent budget returns `TimeBudgetExceeded` before any lookup/commit. `cargo test --lib sync::reconcile`: 24 passed, 0 failed. |
| I1 | 256 MiB file / 32 GiB import / 256 MiB query value: can the documented maxima be processed within explicit memory/runtime budgets? | **SQLite clamp resolved** (`7d87f04`); memory architecture **decision pending** | `max_total_import_bytes` is a sum-of-file work budget, not a memory bound: Audit's 32 GiB import has a ~36 GiB floor (32 GiB content + 8 in-flight × 2 × `max_file_bytes`) and ~64 GiB worst case before parse transients; Adaptive's floor is 16.8 GiB on an 8 GiB host. SQLite defect closed: `max_file_bytes`/`max_query_value_bytes` are clamped to `SQLITE_MAX_VALUE_BYTES` (1,000,000,000) in Adaptive, env overrides, and project config, with a warning; a file in (1e9, old max] now becomes a skipped row. Still open: the memory architecture — architect recommends D2 (hash-first, single-transaction windowed publish) with five parameters recorded in PLAN; not implemented. Residual: lossy UTF-8 can expand content up to 3×, so the content check + `content_limit` skip reason (D2 parameter 3) remains open. |
| I2 | BLOB clone and hex expansion before page-budget accounting | **resolved** (`4d50e18`) | `read_row` now checks TEXT/BLOB lengths with `Row::get_ref` before the owned copy; `execute_and_collect` computes a conservative serialized-size lower bound before conversion, sends an over-page first row straight to `clip_row`, and ends the page before converting an over-page later row. Rows that fit are byte-identical. `cargo test --lib tools::query`: 56 passed, 0 failed. Remaining bounded cost: an admitted value still needs one owned copy for clipping (≈2× live cell bytes); a rejected value no longer allocates. |
| I3 | `compare_versions`: does it promise SemVer, and does it satisfy that contract? | investigation complete — **contract met; no change** | The doc and tests promise only dot-separated numeric comparison, `v` prefix, missing components = 0, and explicitly reject pre-release suffixes (`src/update_fetch.rs:137-173`, `src/update_fetch_tests.rs:27-69`); nothing promises SemVer. The update flow uses `/releases/latest`, which excludes pre-releases (`src/update_fetch.rs:74-80`). Residual process risk only: a pre-release/build-metadata tag published as a normal release would be fetched; not a code-contract violation. |

### Commits on this branch

- `b199277` — bounded util byte read, including the latent unbounded read in `read_file_capped`
- `dd87904` — 1 MiB `.gitignore` cap, `not_written` behavior
- `94726ab` — streaming hash cap, bounded sampling, 4096-path reconcile batching
- `4d50e18` — query `get_ref`/lower-bound pre-check
- `7d87f04` — watch scope caps, lock-free walk, `Partial`/`degraded`, SQLite value clamp
- `1f17dc5` — source-limit annotations for the grown files
- `a1147f9` — scope refresh at the start of full verification
- `4f58379` — rustfmt
- `335abfa` — clippy lifetime fix

## Limits and behavior

- **`.gitignore`**: 1 MiB `MAX_FOREIGN_TEXT_BYTES`; over-cap/unreadable → `GitignoreOutcome::NotWritten`
  with the observed error; missing file still creates the rule; the project remains enabled.
- **Hashing**: `max_file_bytes`; `take(cap + 1)` streaming; no byte past the cap is hashed;
  `HashError::TooLarge` → skipped existence row.
- **Sampling**: the same cap through `read_file_capped_bytes`; `SampleError::TooLarge` carries the
  observed length.
- **Reconcile stored-hash lookup**: 4096-path batches, one JSON parameter per batch; the `Deadline`
  is checked before every batch and the final flush; a spent budget → `TimeBudgetExceeded`, no commit.
- **Query rows**: `max_query_value_bytes` checked with `Row::get_ref` before the owned copy; a
  conservative serialized-size lower bound sends an over-page first row to `clip_row` and ends the
  page before converting an over-page later row; fitting rows are byte-identical.
- **Watch scope**: `max_watch_entries`/`max_watch_depth` (Lean 50k/32; Audit and Adaptive 200k/64;
  process/env only, not project config); the walk and `IgnoreRules` build run outside the manager lock;
  a truncated scope reports `WatcherHealth::Partial` / phase `degraded`; the stat sweep remains the
  freshness guarantee; full verification refreshes the scope before draining.
- **SQLite value cap**: `SQLITE_MAX_VALUE_BYTES` = 1,000,000,000 (bundled `libsqlite3-sys` 0.35.0
  default `SQLITE_MAX_LENGTH`); file/query-value limits are clamped in Adaptive, env overrides, and
  project config, with a warning when an override is reduced.
- **Import budget**: `max_total_import_bytes` is documented as a work budget, not a memory bound.

## Validation

- `cargo fmt --all -- --check`: exit 0.
- `check_source_limits`, `check_no_duplicates`, `check_docs_drift`, `check_producer_coverage`,
  `check_governance`, `check_mcp_surface`, `check_version_consistency`: all PASS.
- `cargo clippy --all-targets --all-features --locked -- -D warnings`: exit 0.
- `cargo test --lib --bins --tests --all-features --locked --no-fail-fast`: **1033 passed, 0 failed,
  2 ignored**.
- `cargo deny check`: advisories/bans/licenses/sources ok. `cargo machete`: 0 unused.
- `cargo build --release --locked`: 30,399,448 bytes.
- `check_coverage`: **90.20%** (10,357 / 11,482 lines), gate 83%.
- `cargo test --test large_repo_soak --locked -- --ignored --nocapture`: 10,000 files published and
  verified in 19.940 s, budget 1200 s; 1 passed.
- Targeted suites: activation 30, hash 11, sample 19, reconcile 24, query 56, watch 57, limits 11,
  profile 8, manager 37, health 31.
- MCP surface re-recorded (7 insertions / 1 deletion: `partial` health variant, `degraded` phase);
  `check_mcp_surface` PASS.

## Remaining risks / architectural decisions

- **Import memory (owner decision):** D2 recommended, not implemented. Current Audit floor ~36 GiB,
  worst ~64 GiB; Adaptive 16.8 GiB floor on an 8 GiB host. The five binding parameters are in PLAN.
- **Lossy UTF-8 content limit:** a file under the file cap can still exceed SQLite's 1e9 value cap
  after 3× lossy expansion; D2 parameter 3 (content check + `content_limit` skip reason) is open.
  Until then, a very large invalid-UTF-8 file in the Adaptive profile can still fail the publish at
  insert.
- **Grammar notices:** not applicable; the hybrid static bundle was reverted 2026-10-19. All languages
  use the pack's runtime download, per the 2026-10-18 accepted-risk decision.
- **Watch:** the full-verification scope refresh closes the transient `rules_for == None` window;
  the caps are process/env only.
- **Query:** an admitted max-size value still needs one owned copy (~2× live bytes) for clipping;
  rejected values no longer allocate.
- **Source-size exceptions:** util 230, hash 243, pipeline 238, query_exec 210, watch/manager 295 —
  all in the project's 200–300 slugthug band, annotated.
- **Remaining audit waves:** reconciled 2026-10-19 against `f9544ca` — **441 open** (R5 149,
  R8 192, R7 22, R12 17, R10 23, R9 24, R6 12, R4 2), **28 handled**, **13 not violations**,
  **2 needs decision**. See `.planning/audit_2026-10-18_triage.md`; the ledger states are updated.
