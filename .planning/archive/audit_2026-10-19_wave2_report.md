# Wave 2 — production-state completion report

Repo: `/home/slugaudit`, `main` after the wave-2 cherry-picks and the final
surface snapshot. Base before wave 2: `6241b43` (deliberate doc deletion).
Date: 2026-10-19.

## What wave 2 closed

### Fresh canonical docs (not restored from git)

Commit `7a782de` authored new documents from current behavior:

- `.planning/PLAN.md` — active plan with explicit states.
- `.planning/archive/DECISIONS.md` — append-only decision log.
- `docs/CLIENTS.md` — all connect targets, config paths, scopes, verification.
- `docs/TROUBLESHOOTING.md` — operator runbook.
- `docs/RELEASING.md` — release artifacts, checksums/SBOM/attestation,
  consumer verification.
- `docs/about.toml` + `docs/third-party-notices.hbs` — cargo-about config and
  template matching `deny.toml` and the release workflow.
- `326ec9a` corrected AGENTS.md's parser-pack path to `src/parse/mod.rs`.

`check_docs_drift` passes. `cargo-about` is not installed here, so the config
and template were syntax-checked and matched to cargo-about 0.9.2, not rendered.

### MCP surface

`5d77856` recorded the final `.planning/archive/mcp_surface.json` from the
integrated tree: 8 tools, 8 resources, including the `finding_read` paging
fields, the persisted `unrepresentable_path_count` description, and
`HealthPhase::unknown`. `check_mcp_surface` passes.

### Report count without a per-call walk

`b19f4a0` added a disposable `unrepresentable_paths (path BLOB PRIMARY KEY)`
table and bumped `CURRENT_SCHEMA_VERSION` 7 → 8 (existing databases rebuild).
Full publish discovery and the healthy-path stat sweep replace the set
exactly, including removals; the public `reconcile`/drain paths persist the
sweep's set. `report` reads `COUNT(*)` inside the verified read transaction and
no longer calls `sync::discover`. Tests cover add → 1 → remove → 0 through full
and watcher-backed syncs. `index_partial` remains `count > 0 || skipped > 0`.

### CLI fallible output and failure arms

`bac3d0a` + `1b2b671`:

- `cli::out` / `cli::err` / `write_to` / `is_broken_pipe`; every production
  `print!/println!/eprint!/eprintln!` in the CLI/connect/install/update files
  replaced with fallible writes/flushes.
- `BrokenPipe` exits cleanly with status 0 and no error log; other write errors
  propagate through the module's typed error.
- `ConnectError::Io` / `InstallError::Io` expose the io error through
  `source()` (Display unchanged) so the chain walk can classify BrokenPipe.
- `CurrentExe`, `Clock`, `BinaryPath`, prompt-flush, and platform predicates
  are tested through pure helpers; no production test hooks.
- `lib.rs`'s stale print_stdout comment corrected.

End-to-end: menu output piped into `head -c 10` exits 0 with empty stderr;
`slugaudit help >/dev/full` exits 1 with the io error.

### Remaining failure arms / dead API

`ed58a4b`:

- Stale-metadata discard test via `pub(super)` recovery helper and a valid
  stale DB with a directory at the database path.
- Netfs positive rejection via a pure `reject_if_network(path, on_network)`
  verdict, with a positive and negative control.
- Context read-transaction start test with an active outer transaction.
- Project-control serialize arm removed by constructing the `Value` directly;
  discard/reopen tested through a recovery helper with directory/missing-parent
  fixtures.
- `SyncError::Revision` and `SyncError::Read` removed (provably unused).
- `DiscoveryError::Walk` **retained**: it is constructed at
  `src/sync/discovery.rs:180` as a skip reason; the first-wave R5 premise was
  wrong.
- Connection Configure error arms (`busy_timeout`/function registration)
  accepted as unreachable without an invalid handle/OOM; the 5 s timeout
  behavior remains tested; the acceptance is documented at the arm.

### Performance baseline

`6728735` recorded 12 benchmarks to `.planning/archive/perf_baseline.json`
using the default `target/` (the gate hardcodes `target/criterion`). The
comparison passed: every measured ratio was between 0.98x and 1.14x, threshold
20%. The benchmark groups are discovery (4), parsing (4), and search (4).

## Validation — full gate, all pass

`main` at `6728735`, clean `target-final`:

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

## Cleanup

Pending: remove the five wave-2 worktrees/branches, the private hardlink target
clones, `target-base`, `target-final`, and the default bench target. Main is
local and not pushed; the docs are now committed as fresh files.
