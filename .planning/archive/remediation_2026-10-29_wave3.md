# Remediation wave 3 (2026-10-29), base 50db153

| Item | Status | Commit |
|---|---|---|
| R1 --help on every subcommand | landed | 188873b |
| R2 project_control off | owner ruling: deletion intended. Count added in a214880, then REVERTED; `off` is back to pre-wave behavior (destructive_hint was already true) | a214880, revert 2d777f8 |
| R3 backup on discard | HELD by owner (not implemented) | - |
| R4 mountinfo unreadable | BLOCKED: statfs needs `unsafe` but `unsafe_code = "forbid"` (Cargo.toml:99, src/lib.rs:15, src/main.rs:9); awaiting owner ruling (lift forbid / rustix / stopgap) | - |
| R5 real path + error in store/sweep WARNs | landed (other `redacted_path` sites, ~20, untouched by scope) | ce305dd |
| R6 no WARN for non-JSON configs | landed | c32247f |
| R7 menu re-prompt without banner | landed | 31d6030 |
| R8 skip sweep when Healthy+quiet, 30 s backstop | landed with caveats below | d0c7d86 |
| R9 connect: config-only enumeration, 15 s CLI timeout | landed | 4395fec |
| source-limit split (R6/R7 side effect) | landed | 62f5a23 |

## R8 caveats
- Edit-then-query before the OS event is delivered can miss the change until the next call or the 30 s backstop (src/sync/full_sweep_gate.rs, manager_healthy.rs).
- With pending events the call still walks the full tree; the win is idle/read-only call chains.
- Non-UTF-8 name count can lag up to 30 s.

## Gate (integrated HEAD d0c7d86, before report commit)
README Development gates all rc=0; tests 1584 lib + integration all pass; coverage 92.72% (gate 83); check_performance PASS (no regression >20%); cargo deny: advisories/bans/licenses/sources ok, 0 warnings.
Release smoke: `update|connect|serve --help` print usage, rc 0, HOME left empty; `connect` returns immediately, no JSON WARN. Not run: off-with-findings smoke (covered by tests; R2 changed), no-mountinfo smoke (R4 held).

## Closing items
| Item | Commit |
|---|---|
| R5 extension: ~45 remaining log sites log real path/root | 126ebf6 |
| Freshness wording (README, INSTRUCTIONS) | 653953b |
Left redacted: src/tools/paginate.rs:279 (stored result value, not a log); src/store/netfs.rs:246 (R4 rework); src/sync/recovery_log.rs:65 (root from untrusted DB column). redacted_path/redacted_log_id remain because of these.
## Gate on 653953b
fmt, all check bins, check, clippy -D warnings rc=0; 1581 lib tests pass; coverage 92.71% (gate 83); perf PASS; cargo deny ok (0 warnings, no dependency added). Smoke: update --help prints usage rc 0; off tests pass. No-mountinfo smoke not run (R4 blocked).
