# Performance baseline (Task 9.2)

Status: baseline recorded 2026-08-10. Benchmarks live in `benches/`
(`discovery.rs`, `parsing.rs`, `search.rs`, `sync.rs`) with shared fixture
generation in `benches/common/mod.rs`. Criterion is a dev-dependency only;
benchmark builds are separate from correctness gates (plan-audit PHASE-09).

## Machine and environment

Recorded on the machine that produced the numbers below; any future run
that is compared to this baseline must be on a comparable machine or the
machine section must be re-recorded.

- CPU: AMD Ryzen 9 5900X 12-Core (6 cores visible to this environment)
- RAM: 10 GiB
- OS: Linux 6.17.13-3-pve x86_64 (Proxmox container), root filesystem ZFS
- Compiler: rustc 1.97.1 (8bab26f4f 2026-07-14), cargo 1.97.1
- Build profile: `bench` (optimized), `--locked`
- Baseline recorded against HEAD `7d7c872` (notify 8.2.0). The sync
  benches use a watcher-less manager (`SourceSyncManager::new()`), so the
  concurrent notify 7→8.2.0 bump does not affect any measured number.
- Runtime parser cache: warm from prior builds; `get_parser` cold load
  measured at 220 µs (prebuilt grammar, no network fetch required)

## Fixture definition

Generated deterministically from the file count (`benches/common/mod.rs`,
splitmix64-seeded — identical content on every run and machine):

| Fixture | Files | Total bytes | Mix |
|---|---|---|---|
| small | 40 | 20,357 B | 12 Rust + 12 Python + 4 JS + 4 TS + 4 Markdown + 4 JSON + 2 malformed |
| large | 200 | 101,751 B | 60 Rust + 60 Python + 20 JS + 20 TS + 20 Markdown + 20 JSON + 2 malformed |

Every source file imports a project-local sibling (exercising
dependency-edge resolution during publish) and external modules; files
contain file-unique `needle_{n}` symbols and a common `shared_helper`
token used as search needles. The activation marker
`.planning/slugaudit/` is present, so `ensure_current` runs the real
production path.

## Protocol

Baseline command (all four benches, trimmed settings for a quick baseline;
median reported):

```bash
cargo bench --locked --bench discovery --bench parsing --bench search --bench sync \
  -- --sample-size 20 --warm-up-time 2 --measurement-time 10
```

- Discovery/parsing/search benches: 20 samples, 2 s warm-up, 10 s max
  measurement per bench.
- Sync benches set `sample_size(10)` and `measurement_time(8 s)` in code;
  CLI flags above override them.
- Cold parser load is captured once, before any criterion benchmark runs,
  and printed to stderr (`parser_cold_load_rust: ...`).
- Database sizes are printed to stderr (`db_size_after_*_sync_*: ...`).

## Results (2026-08-10, median)

### Discovery

| Bench | Time | Throughput |
|---|---|---|
| walk_small (40 files) | 718 µs | ≈ 56 k files/s |
| walk_large (200 files) | 1.93 ms | ≈ 104 k files/s |
| walk_and_hash_small | 1.09 ms | ≈ 19 MB/s content |
| walk_and_hash_large | 3.62 ms | ≈ 28 MB/s content |

Walk includes the 8 KB binary sniff per file. Hashing dominates the
walk-and-hash pipeline.

### Parsing (warm parse + evidence extraction, ~50-line samples)

All four grammars are force-loaded before any benchmark runs, so every
number below is genuinely warm-cache parse + extraction (the rust cold
load is captured separately as `parser_cold_load_rust`; a later run
measured 143 µs, so treat the cold figure as < 1 ms with OS-cache
variability).

| Bench | Time |
|---|---|
| cold parser load (rust) | 220 µs |
| extract_rust | 1.33 ms |
| extract_python | 763 µs |
| extract_javascript | 522 µs |
| extract_typescript | 567 µs |

### Search (read-only SQL against a synced database)

These measure the SQL execution layer only — the full `query` tool adds
`ensure_current` freshness verification, JSON row serialization, and the
row/byte caps on top.

| Bench | Time |
|---|---|
| substring_like_small (LIKE over content) | 12.5 µs |
| substring_like_large (LIKE over content) | 40.4 µs |
| symbol_lookup_large (evidence kind + payload LIKE) | 254 µs |
| dependency_traversal_large (recursive CTE) | 16.4 µs |

Search is a bounded-scan (`LIKE`) over `files.content` /
`evidence.payload` — there is no FTS5 table (see §21.4 "FTS5 versus
bounded-scan search"). 40 µs for a full-content scan of 200 files is well
inside budget; FTS5 becomes the option to revisit if this grows
super-linearly on much larger trees.

### Sync (through the real `SourceSyncManager::ensure_current`)

The manager is created with `SourceSyncManager::new()` — no watcher — so
**unchanged/changed sync numbers are the worst case**: every call takes the
full-verification publish path (discover + hash everything + diff). A
deployment with a healthy watcher skips the walk when nothing is dirty;
that path is covered by functional tests, not benchmarked here.

| Bench | Time |
|---|---|
| first_sync_40 | 45.1 ms |
| unchanged_sync_40 | 1.99 ms |
| changed_file_sync_40 | 9.30 ms |
| first_sync_200 | 160.7 ms |
| unchanged_sync_200 | 5.07 ms |
| changed_file_sync_200 | 10.2 ms |

First sync of 200 files ≈ 1.2 k files/s, dominated by per-file parse
(~0.6–1.3 ms each, matching the parsing benches). Unchanged sync re-walks
and re-hashes but publishes nothing. Changed-file sync re-parses exactly
the touched file and is nearly size-independent (9.3 ms vs 10.2 ms).

### Database growth

| Size | After first sync | After unchanged | After ~20 changed iterations |
|---|---|---|---|
| 40 files | 249,856 B | 249,856 B (no growth) | 851,968 B |
| 200 files | 905,216 B | 905,216 B (no growth) | 1,327,104 B |

Unchanged sync writes no rows (manifest-identical publish is skipped), so
the database does not grow. Changed sync replaces one file's evidence and
appends one revision row per iteration (~25–30 KB/iteration including WAL
churn); WAL checkpoints on connection close.

## Budgets (proposed targets)

Proposed budgets for the release gate, pending confirmation on a
representative real repository. The 200-file fixture is the benchmarked
proxy; budgets scale with fixture size.

| Operation | Budget |
|---|---|
| Startup (process → ready for first tool call) | < 1 s (not yet benchmarked) |
| Cold parser load | < 5 s (measured 220 µs with prebuilt grammar) |
| First sync, 200 files | < 30 s (measured 161 ms) |
| Unchanged sync, 200 files | < 2 s (measured 5.1 ms worst-case walk) |
| Changed-file sync (1 file) | < 1 s (measured 10.2 ms worst-case walk) |
| Search query (any) | < 500 ms (measured ≤ 254 µs) |
| Evidence retrieval (symbol lookup) | < 500 ms (measured 254 µs) |
| Dependency traversal (CTE) | < 500 ms (measured 16 µs) |
| Memory, 200-file fixture | < 512 MB peak (not yet benchmarked) |

## Regression policy

- Comparisons use median time per bench on the same machine class and
  fixture definition, with the exact command recorded above.
- A "statistically meaningful regression" is one where the new median
  exceeds the recorded median by ≥ 20% (or the low bound of the new run
  exceeds the recorded high bound) for the same fixture and machine class.
- Failures of the budgets above block release; benign noise within the
  thresholds does not. Sync/parse/benchmark runs are excluded from the
  correctness gates by construction (separate build targets).

## CI regression gate

`tools/check_performance.sh` is wired into CI and fails the build when any
bench regresses more than the threshold against the committed
machine-readable baseline (`.planning/perf_baseline.json`, derived from the
tables above; medians in nanoseconds, release budgets where they map).

- **Command**: `bash tools/check_performance.sh [threshold-percent]`
- **CI protocol**: the four benches run with a reduced sample size
  (`--sample-size 10 --warm-up-time 1 --measurement-time 3`) so the gate
  stays ~3–5 min; the recorded baseline used the longer protocol (20/2/10).
  The reduced-sample run is noisier, which is why the 20% regression
  threshold — not the tighter 10% — is the gate.
- **Output**: a per-bench table of baseline / measured / ratio / verdict,
  with `PASS` or a `FAIL:` line per regression (median ratio > 1 + 20%, or
  a measured median over its recorded budget).
- **Re-recording**: baselines are machine-class specific. When the machine
  or protocol changes, regenerate and commit the baseline deliberately
  (`bash tools/check_performance.sh --record`), update this file, and
  record the change in `DECISIONS.md` — do not silently re-record to hide
  a regression.
- **First gate run (2026-08-10)**: PASS against the baseline on the
  recording machine; worst ratio `sync_40/first_sync` 1.14x (noise),
  everything else ≤ 1.03x, several benches measured faster
  (`walk_small` 0.82x). The wall-clock timeout instrumentation added
  after the baseline caused no measurable regression.
- **Observed variance**: the reduced-sample protocol on the fast benches
  swings roughly ±25% run-to-run on unchanged code (e.g. `walk_small`
  0.82x, `changed_file_sync_40` 0.76x in the same run) — that is noise,
  not signal. If a red run's ratios cluster around 1.15–1.30x on the
  sub-ms benches only, treat it as noise and re-run before re-recording;
  a regression that matters also shows up on the ms-scale sync benches,
  where variance is a few percent. A bench that produces no measurements
  fails the gate (a silently-skipped group is exactly what the gate
  exists to catch).

## To re-run

```bash
cargo bench --locked --bench discovery --bench parsing --bench search --bench sync \
  -- --sample-size 20 --warm-up-time 2 --measurement-time 10
```

Capture `time:` medians and the `db_size_after_*` lines, update this file,
and record a dated entry in `DECISIONS.md` for any change in machine or
protocol that invalidates direct comparison.
