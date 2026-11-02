# SlugAudit plan

Last updated: 2026-11-02. The owner approves every step before it is
committed. AGENTS.md is the product definition; this file is the work order.

## Goal

Make the tool accurate and fast for Rust: every answer matches the code on
disk at the moment of the call, safely, on repos up to Photoshop size (~40k
files) on a normal laptop. Nothing else gets built until that is true.

## Committed on main (2026-11-02)

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

## Gate on `04f3c8c`

- `cargo fmt --all --check` — pass
- `cargo clippy --workspace --all-targets -- -D warnings` — pass
- `cargo test --workspace --all-targets` three times — 1619 passed, 0 failed
- `check_mcp_surface`, `check_no_duplicates`, `check_producer_coverage`,
  `check_source_limits` — pass
- `check_coverage` — 92.63% lines (gate 83%)
- `check_performance` — 12 benches within 0.75x–1.02x of `b1ddbb2`
  (20% threshold)

## In flight

Three tool-response contract changes, then the full gate and a commit:

- `project_control.status` becomes a closed enum (`enabled`, `disabled`,
  `already_disabled`). Wire values unchanged; the schema declares them.
- `InvalidMechanic.index` and `InvalidMechanic.id` become required. Wire
  values unchanged.
- `report.parser_failure_files_total` is removed; `parser_failure_count`
  stays. A count larger than `parser_failure_files.len()` means the list
  was truncated.

## Engineering work still open (checked against `04f3c8c`)

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

## Open decisions (owner)

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

## Reference numbers (Ryzen 9 5900X, 32 GB, ZFS NVMe, warm cache)

Per file, 1 thread, stat + open + read + BLAKE3: ~11 µs at ≤4 KB, 54 µs at
64 KB, 0.5 ms at 1 MB, 20 ms at 32 MB. A 10k-file Rust repo costs ~0.2 s per
call single-threaded. Linux-kernel size (82k files) costs ~1.9 s.
