# Remediation Wave 2 (close 1.1.0) - 2026-10-28
Branch main, 4a70544 -> a868d87 (12 commits, local only; not pushed, not tagged). Report by Lead.
Opus was NOT used. W1 failed review twice (see W1); the second failure was fixed by a narrowly briefed Sonnet pass instead.

## Packages
| Pkg | Status | Landed as |
|---|---|---|
| W1 project.db activation marker | LANDED after 3 mason passes + 2 luna reviews. Active iff `.planning/slugaudit/project.db` is a regular file. `off` removes db, sidecars, `results/`, lock; keeps `config.json`; removes the dir only if empty; response has `removed` / `config_kept`. `on` order (dir, db, import) was already correct. Also: result-store slot reset on `on` (off->on paging was broken), non-creating sync open (in-flight call no longer resurrects the db after `off`), `unwatch` only after a successful `off`, symlinked `project.db` tests, docs. | c9d2cf2, 0ce0135, 6df64db, 2fa5a16 |
| W2 CLI | LANDED (luna PASS). | 83b32de |
| W2 PS (progressToken panic) | LANDED (luna PASS). Quinn REPRODUCED the panic on 4a70544 in 4/4 stdio variants (`-32603 ... a scoped thread panicked`; stderr `there is no reactor running` at src/server_runner.rs:310 and `a scoped thread panicked` at src/sync/sample_batch.rs:107) and confirmed it gone on the fix (4/4 variants succeed, 6-9 progress notifications each) plus the unit test. | 17835d9 |
| W2 FC (wave-1 blocker 3) | LANDED after one mason pass (luna FAIL then PASS). First pass had `process_limits()` calling `std::process::exit` in library code (clippy::exit). Now `process_limits()` returns `Result`, limits threaded through watch layer, `PublishError`/`ReconcileError::ProcessLimits`. Squashed to one commit. Follow-up a0f35fe: the fixed 16 GiB `Default()` made `multiple_overrides_compose` host-dependent and poisoned the env lock on this 8 GiB host (14 failures); the test now compares with the host-resolved baseline. | 06c07ef, a0f35fe |
| W2 LG (wave-1 blocker 4) | LEFT OUT per directive (failed review twice). Branch rem2/LG (e2cafbc, 9071645) is complete except `src/tools/health.rs:283,294,305` still log `database_id = redacted_path(..)`. Everything else reviewed OK. Blocker 4 stays open. | - |
| W3 CHANGELOG [1.1.0] | LANDED. Entry dated 2026-10-28. Items for W1, CLI, PS, FC included; LG not (left out). `check_version_consistency`, `check_docs_drift` pass. | 5b7ff14, cbdaf7d |
| W4 planning records | LANDED. PLAN.md Phase 8 state, current state to main, recorded-not-scheduled items (global config, mechanics file, deferred H1/M2/M4/M5 with E1-E5 pointers, plus the review leftovers below), DECISIONS.md entries. MODULE_MAP.md untouched (drift gate passes). | 4260458 |
| W5 housekeeping | DONE. 26 worktrees removed (none had a `target/`: all share /home/slugaudit/target). 19 branches merged into main deleted, plus lane/9.3-leads (tip 8cb5a17, force). 9.3-mechanic had an uncommitted edit to src/tools/golden_phase9_leads_tests.rs: saved as scratchpad `9.3-mechanic-uncommitted.patch` (1430 B) before removal. | - |
| Gate fix (not in directive) | Benches create only the activation directory, so `check_coverage` (runs benches) and `check_performance` panicked `project_not_enabled`. Fixed in `benches/common/mod.rs`. | a868d87 |

Deleted branches (tip): fix/deep-dive-2026-10-25 0508dca; lane/9.3-finding_read 5a6ba1e; lane/9.3-mechanic d3c9f13; lane/9.3-query e31840a; lane/9.3-structure 4fa0f0e; lane/9.4-proofs 07049b2; lane/9.5-surface 525f1b0; lane/sb-leads e8e6fc5; lane/sb-query 6f51188; lane/sb-struct 025060b; rem/P1 8ceb1a0; rem/P3 10209ef; rem/P4 3e0c8aa; rem/P5 8f24458; rem/P6 c098b49; rem/P6b d12c3be; rem/P7 f32e003; rem/P8 c226694; rem2/fc3 a0f35fe; rem2/bn a868d87 (ff-merged); lane/9.3-leads 8cb5a17 (-D).
Kept (not `--merged` because their content was cherry-picked, so `git branch -d` refuses): rem/P2 (superseded), rem2/CLI, rem2/FC, rem2/LG (unlanded), rem2/P2, rem2/PS, rem2/cl, rem2/docs, wave-a-lane-y, wave-c-c4. Fable decides.

## Commit-message rule
Mid-wave the owner banned Co-Authored-By / generated-by lines and model or agent names in commit messages. Every commit above carries none (`git log --format=%B 4a70544..main | grep -ci 'co-authored\|claude\|generated with'` prints 0). Source branches rem2/CLI, rem2/PS, rem2/FC, rem2/P2 had trailers; they were cherry-picked with rewritten messages, so main has none. The unlanded leftover branches still carry them.

## Blockers / decisions for Fable
1. LG left out (above): 3-line fix + a capture test at src/tools/health.rs:283,294,305 would close it. Needs your word; the directive said leave it out.
2. `project_control off` fails on a corrupt `project.db`: `acquire_database_lock` (src/project/activation_lock.rs) opens the file to lock it, so an unopenable db cannot be turned off. Recorded in PLAN.md `still open`; needs an owner decision (let `off` unlink an unopenable db?).
3. Residual window: `off` landing after the sync rebuild path's own discard and before its creating open can recreate `project.db` (both the corrupt and stale-metadata rebuilds). Closing it needs one lock across discard+create. Documented in ARCHITECTURE.md activation section; PLAN.md `still open`.
4. `fuzz/fuzz_targets/support.rs:185` still creates only the bare activation directory; the fuzz targets are probably broken the same way as the benches were. Not built here (separate workspace, nightly). One-line fix (write the marker file like benches/common/mod.rs). NOT recorded in PLAN.md.
5. Shared build dir hazard (owner's /home/slugaudit/.cargo/config.toml target-dir): worktrees and main share artifacts; this wave saw stale test binaries repeatedly (mason r2-LG, quinn, mason W1, and my own gate: run 2 on a868d87 had 18 failures because the lib test binary carried a CARGO_MANIFEST_DIR from a removed worktree). `cargo clean -p slugaudit-mcp-rust` fixed it and the gate was rerun in full. Tradeoff: the shared dir serialises builds (owner's intent) but makes per-worktree results untrustworthy; builders' green fast checks should be treated as provisional.
6. FC residue (PLAN.md `not scheduled`): `discover()` ignores `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` (src/sync/discovery.rs ~268, tests only call it); no tool-level test for the uninstalled-limits error wiring. CLI residue: `serve`, `help`, `update`, `version`, `--help` still ignore trailing arguments (src/cli.rs ~63-73).
7. Flaky test reported by one mason: `tools::finding_read::paged_tests::paged_finding_read_new_finding_after_page_one_is_not_in_the_stored_result_but_in_a_fresh_request` and "database mode 100644 is broader than owner-only 600" umask races. Could not be reproduced by a later worker (5/5 pass) nor in my 3 gate runs. Unconfirmed.
8. Pre-existing on this 8 GiB host (earlier reports): six `query_clip_tests` failed in one integration run at 4a70544; not reproducible later (all 14 pass alone and in the full run). Cause unknown, possibly the env-lock poison cascade.

## Gate (a868d87, tree clean, run by Lead)
All exit 0: `cargo fmt --all -- --check`; check_source_limits; check_no_duplicates; check_docs_drift; check_producer_coverage; check_governance; check_mcp_surface; check_version_consistency; `cargo check --all-targets --all-features --locked`; `cargo clippy --all-targets --all-features --locked -- -D warnings` (0 warnings); `cargo test --lib --bins --tests --all-features --locked`: 1724 passed, 0 failed (lib 1556); `cargo test --test stdio_protocol --locked`: 4 passed; `cargo build --release --locked`.
`check_coverage`: line coverage 92.70% (16876 / 18205), gate 83%.
`check_performance`: PASS, no benchmark regressed more than 20% (highest ratios seen in the table tail were 0.92x and 0.90x of baseline, i.e. faster; threshold 20%).
Not run: cargo-deny, cargo-mutants (CI-only). This report's own commit is docs-only and was added after the gate.
Earlier invalid runs, for the record: gate run 1 on cbdaf7d failed `check_coverage` and `check_performance` (benches, fixed by a868d87); run 2 on a868d87 failed 18 lib tests (stale binary, above).

## Smoke (release binary built at a868d87; driver scratchpad/smoke/smoke.py)
initialize ok; `project_control on` on a 20-file fixture: enabled, 21 files imported; `mechanic add` (persist) ok; `report` ok; `off`: removed project.db and sidecars, only `config.json` left in `.planning/slugaudit/`; `report` after `off`: error -32602 `project_not_enabled`, `project.db` NOT recreated; `on` again: enabled; `mechanic list` shows `smoke_unwrap` with source `project_config` (restored from config.json); stdin close: exit 0.

## Cut / not done
LG (above); `off` on a corrupt db; closing the rebuild-path window; fuzz target fix; the unscheduled residue in PLAN.md; deleting rem2/* branches whose content landed by cherry-pick; cargo-deny / cargo-mutants; no push, no tag.
