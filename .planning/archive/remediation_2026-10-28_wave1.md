# Remediation Wave 1 + Evidence Wave — 2026-10-28
Branch fix/deep-dive-2026-10-25 @ 82f24a4 (from 4477552). Not pushed. Report by Lead.

## Packages
| Pkg | Status | Landed as |
|---|---|---|
| P1 C2 | LANDED. Startup no longer deletes; prior-session rows marked stale; reads default to current session; new tool `finding_sessions` (list/adopt; adopt restores `current` only if source_hash still matches). Explicit physical cleanup NOT implemented. | rem/P1 8ceb1a0 |
| P2 C3 | BLOCKED (commit 5a16961 on rem/P2, not merged). See Blockers. | - |
| P3 C4 | LANDED. `database_discard {findings_discarded, cause}` on the discarding call; null when DB unreadable; one WARN. | rem/P3 c09ae07, 10209ef |
| P4 C1+H3+H2 | LANDED. Real path + error=%source at sync sites; default filter `warn,slugaudit_mcp_rust=info,rmcp=error`; error message logged once in log_completion; redaction contract narrowed. `redacted_path` NOT deleted (~90 other callers). | rem/P4 3e0c8aa |
| P5 H4+H5+H6+M14 | LANDED. Malformed project config fails the call; invalid SLUGAUDIT_*/RUST_LOG exit 1 naming the variable; undetectable RAM requires explicit profile; health `system_memory` nullable. | rem/P5 8f24458 |
| P6 H7+H8+M11 | LANDED + P6b fix. Exit 2 on usage errors, Display errors, -v, SIGTERM/SIGINT clean exit 0, pre-initialize EOF exit 0. New dep signal-hook-registry 1.4.8 (tokio `signal` feature). | rem/P6 c098b49, rem/P6b d12c3be |
| P7 H9+M6+M9+M7+M8 | LANDED. Append+fsync rc; unknown $SHELL refuses; atomic .prev; install.sh trap/printf, cargo fallback now installs to TARGET_PATH; malformed asset entry errors, partial download removed. | rem/P7 f32e003 |
| P8 M12+M10+M13 | LANDED. preserve_order, mode preserved, trailing newline, NO_COLOR/TERM=dumb, gitignore dir fsync (lock skipped). Commented client configs still refused. | rem/P8 c226694 |

## Blockers / decisions for Fable
1. P2: `find_project_root` (src/project/activation.rs:125-145) treats an existing `.planning/slugaudit/` dir as "active". Keeping config.json (owner decision) leaves the dir, so after `off` the project is still active and the next call recreates the DB. Also `results/` (server runtime state) keeps the dir non-empty and fails `result_store_proof_project_control_off_keeps_the_open_result_readable` (src/sync/result_store_proof_replacement_tests.rs:273). Options: key activation on project.db (recommended; unverified whether `on` creates the dir first), or delete everything but config.json.
2. New defect (E3): `query` with a progressToken panics at src/server_runner.rs:311 (tokio::spawn from std scoped thread, src/sync/sample_batch.rs:107), -32603. Not in scope; not fixed. Reported by quinn trace, not reproduced by me.
3. P5 leftovers: process_limits()/ResourceLimits::default() log-and-fallback outside startup validation; zero limits still warned-and-defaulted; non-UTF-8 SLUGAUDIT_LOG_FORMAT only logged.
4. P4/P3 leftovers: log_notify_failure callers still pass hashed root_id; P3's recovery WARN redacts the db path (10209ef), contrary to the P4 real-path policy.
5. install.sh cargo fallback target changed to TARGET_PATH.

## Gate (82f24a4)
build --release --locked pass; clippy --all-targets --all-features --locked 0 warnings; cargo test --lib --bins --tests: 1717 passed, 0 failed; fmt pass; check_source_limits, no_duplicates, docs_drift, producer_coverage, governance, mcp_surface, version_consistency all exit 0. Not run: check_coverage (needs cargo-llvm-cov), check_performance (local gate).

## Smoke (out/smoke3 vs out/smoke)
Startup stderr 2,558 B, only our lines (was rmcp INFO peer_info dump). Failed call: one WARN with error text, no rmcp duplicate. --bogus exit 2 (was 1); -v works; pre-initialize EOF exit 0 with one line (was exit 1 ConnectionClosed Debug); SIGTERM exit 0 immediately (first smoke on 96aeb0a hung >15 s, fixed by P6b). RUST_LOG=debug still 160 KB (M5 left alone). Stdout clean JSON-RPC, first response ~5 ms.

## Evidence
| E | Result |
|---|---|
| E1 H1 | `health` ~0.1 ms (no path) / 2, 6, 52 ms (5k/10k/100k, path). `query`/`finding_read` with no changes 0.5-2 s p50 (748/713/1345 ms at 5k/10k/100k), weak scaling; load 19-28, n=10, no p99. 100k import 72 s, RSS 386-431 MB. Cause unprofiled: INCONCLUSIVE. |
| E2 M2 | 0 Busy in 100 A attempts. 95 of 520 calls failed with JSON-RPC -32603 "concurrent publish won the race ... retry the call", up to 8.6 s. Real symptom is the CAS race, not Busy. |
| E3 M4 | Pipe full at 2 s, completions stuck at 74 of 1476; RSS +0.5 MB/s (7->33 MB over 28 s); recovered after read resume (46 MB). Run only hit the panic error path (new defect above). |
| E4 M5 | Stalls: default filter at call 143 (`health`, ~437 B/call) or 381 (unknown tool); debug stalls during startup. Drain releases it. Confirmed. |
| E5 M12 | config.json keys re-sorted alphabetically on rewrite, no trailing newline (before P8). Claim "order preserved" wrong. |
Scripts: scratchpad/evidence/E1..E5.

## Cut / not done
P2 (blocked), explicit cleanup action for P1, redacted_path deletion, in-process HTTPS client, multi-platform ASSET, install/menu extra-arg rejection, deferred H1/M2/M4/M5 fixes.
