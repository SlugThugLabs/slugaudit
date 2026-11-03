# SlugAudit plan

Last updated: 2026-11-03. The owner approves every step before it is
committed. AGENTS.md is the product definition; this file is the work order.

## Goal

Make the tool accurate and fast for Rust: every answer matches the code on
disk at the moment of the call, safely, on repos up to Photoshop size (~40k
files) on a normal laptop. Nothing else gets built until that is true.

## Committed on main (2026-11-03)

- `ec7e2cf` — AGENTS.md: the MCP server process (start, stdio, lifecycle,
  shutdown) is the spec's and `rmcp`'s job, not SlugAudit's.
- `77e6588` / `590b2fd` — every `saturating_*`/`wrapping_*` in `src/` is
  either a `// KEEP saturating|wrapping: <reason>` or converted to a
  `checked_*` error; `tests/saturating_policy.rs` fails any unmarked use.
- `58f45e1` — restored PLAN.md and the MCP surface snapshot that `79e44f6`
  deleted by mistake.
- `79e44f6` and the `np-g1`..`np-g5` merges — no panics anywhere, tests
  included: clippy denies every `assert*` (`disallowed_macros`),
  `indexing_slicing` and `arithmetic_side_effects`. Tests use
  `ensure`/`ensure_eq`/`ensure_ne`/`check` from `test_check`, and every
  test returns `TestResult`.
- `9f8593a` — the word `unwrap` only where the audit names it; `let _ =`,
  `.ok()`, `map_or` and `Err(_)` fallbacks removed.

## Committed on main (2026-11-02)

- `cfaf1ce` — the three response contracts: `project_control.status` is
  a closed enum, `InvalidMechanic.index`/`id` are required, and
  `report.parser_failure_files_total` is gone.
- `04f3c8c` — close the review's 180 findings and bound tool reads.
  Two-pass bounded sync (hash every file, sample only the changed ones),
  shared input caps, result-store/evidence/leads limits, errors fail closed
  and keep their cause (logged once), PRAGMA refused before preparation.
  The source-limit agent approval tag is `approved-by=slugthuglabs`.
- `b1ddbb2` — no `unwrap`, `expect`, `unwrap_or*`, `panic!` or
  `unreachable!` anywhere; errors are returned instead.
- `3805ca2` — the `check()` test helper crate; one workspace lint table.
- `5763281` — deny `clippy::pedantic`, `missing_docs` and indexing.
- Earlier: pedantic findings cleared, comment cleanup, function splits,
  error-handling fixes, docs build in CI.

Also committed earlier and still standing: the activation switch
(`.planning/slugaudit/` exists = on; `on` creates, `off` deletes, a missing
or broken DB rebuilds) and session-only findings/mechanics/profile.

## Gate on `77e6588` (2026-11-03)

- fmt, clippy `-D warnings` (0), 1636 tests passed / 0 failed, and
  `check_source_limits`, `check_no_duplicates`, `check_producer_coverage`,
  `check_mcp_surface` — pass. Coverage 91.5% at `79e44f6`.
- `check_performance` — `discovery/walk_small` fails at random (±27%
  host noise on this Proxmox container, main included); every other bench
  within 4%.

## Gate on `cfaf1ce`

- `cargo fmt --all --check` — pass
- `cargo clippy --workspace --all-targets -- -D warnings` — pass
- `cargo test --workspace --all-targets` three times — 1619 passed, 0 failed
- `check_mcp_surface`, `check_no_duplicates`, `check_producer_coverage`,
  `check_source_limits` — pass
- `check_coverage` — 92.63% lines (gate 83%)
- `check_performance` — 12 benches within 0.75x–1.02x of `b1ddbb2`
  (20% threshold)

## Engineering work still open (checked against `cfaf1ce`)

- **C5, slimmed:** the mechanic result cache (`mechanic_scan_cached`)
  still exists; `check_performance` and the `criterion` dependency are
  still present. Reduce config to the knobs a user needs, delete the rest.
- **Per-call time limit:** defaults are still 120 s / 1200 s
  (`max_sync_wall_clock`); cut them to tens of seconds.
- **Cursor freshness:** a page reports the revision it was stored from
  (`page.revision_id`); it does not yet report the revision on disk at
  continuation time. Do not fail the page — agents edit while they read.
- **Test cleanup:** tests for removed security layers and the old design,
  and low-value wording tests, are still in the suite.
- **Invariant coverage:** some real-path cases exist (deleted DB rebuild,
  non-UTF-8 paths, deleted files). The remaining edit/add/rename,
  file-becomes-directory and FIFO cases from the old list still need the
  same treatment.
- **Soak timing:** `tests/large_repo_soak.rs` covers a 10k-file import
  inside the wall clock. The unchanged-tree second call and the
  one-edit call timing are not pinned yet.

- **Assert wording:** converted tests whose assert had no message carry
  the expression text as the description (e.g. `"c"`); rewrite as prose.
- **Lenient JSON checks:** a few tests treat a missing key as passing
  (`is_none_or`, the `field()` helper in `src/evidence/mod_tests.rs`).
- **`LineIndex::new`:** stores an overflow error and returns it on every
  query; make the constructor fallible (callers
  `evidence/normalize.rs`, `tools/structure_exec.rs`).

## Open decisions (owner)

- Fuzz targets keep 3 `assert!` (a fuzzer reports a bug by crashing)?
- Logs: add file:line to every log line (`init_logging`, `src/main.rs`)?
- Perf gate: compare the fastest of 5 runs so `walk_small` stops flaking?
- Commit `.planning/archive/perf_baseline.json`, or keep it local?
- UTF-8 skip reason: name the byte where the file stops being UTF-8?

- Coverage gate: keep or drop the CI check on line coverage.
- File-size cap: today 32 MiB. 4 MiB keeps the worst case per file around
  10 ms on a spinning disk.
- Per-call DB tamper check: is re-hashing stored text worth roughly
  doubling the check cost?
- `mechanic` is missing from the `lhm.plugin.json` tool list.
- Three other directories list SlugAudit: what does each one promise?

## Deferred by the owner

- Reinstall SlugAudit and restart the client once the current work is
  worth using; the running server is stale until then.
- Release: `Cargo.toml` says 1.1.0; the last released tag is v1.0.10. Take
  a version up only when there is something worth releasing, then update
  every listing the same day.

## Not doing

- No file watcher.
- No security guards against threats nobody reported.
- No derived-row digest (it cannot catch the bugs that matter).
- No removing other languages or the installer/connect/update/menu surface.
- No new features until the goal above is met.
- No exit-path layer around the MCP process (reply ledger, drain wait,
  forced-exit grace). The `exit-paths` branches were dropped 2026-11-03.

## Reference numbers (Ryzen 9 5900X, 32 GB, ZFS NVMe, warm cache)

Per file, 1 thread, stat + open + read + BLAKE3: ~11 µs at ≤4 KB, 54 µs at
64 KB, 0.5 ms at 1 MB, 20 ms at 32 MB. A 10k-file Rust repo costs ~0.2 s per
call single-threaded. Linux-kernel size (82k files) costs ~1.9 s.
