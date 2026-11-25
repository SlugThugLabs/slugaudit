# SlugAudit Project Plan

Last updated: 2026-11-25. `AGENTS.md` is the product definition; this file
records what is built on `main`, what engineering work is approved, and what
decisions are in draft for the owner. The owner approves targets before work
begins and before anything is committed.

## Goal [approved]

Make the tool accurate and fast for Rust: every answer matches the code on
disk at the moment of the call, safely, on repos up to Photoshop size (~40k
files) on a normal laptop. Nothing else gets built until that is true.

## What the Product Is [built]

- **Local stdio MCP server on `rmcp`:** SlugAudit runs over stdio on the `rmcp`
  SDK (`3.11.0`), which owns process startup, transport framing, session
  lifecycle, and shutdown. SlugAudit provides the SQLite index, the per-call
  sync pipeline, and 10 MCP tools: `query`, `report`, `structure`, `leads`,
  `mechanic`, `finding`, `finding_read`, `finding_clear`, `project_control`,
  and `health`.
- **Per-call disk synchronization:** On every tool call except `health`, before
  answering, SlugAudit walks the project, BLAKE3-hashes every regular file,
  updates changed rows, adds new files, deletes removed files, and re-hashes
  stored `files.content` against `files.content_hash` in
  `<root>/.planning/slugaudit/project.db` (`files`, `evidence`,
  `dependency_edges`, `revisions`). A missing, unreadable, root-mismatched, or
  schema-incompatible database is discarded and rebuilt from disk.
- **Opt-in project switch:** A project is active iff
  `<root>/.planning/slugaudit/` exists. `project_control` with `action: "on"`
  calls `slugplan::ensure` (`v0.1.1`) to create `.planning/` and its shared
  `README.md`, creates `.planning/slugaudit/`, and runs the initial import.
  `project_control` with `action: "off"` deletes `.planning/slugaudit/` while
  leaving `.planning/README.md` and user files in `.planning/`. Nothing else
  creates `.planning/slugaudit/`.
- **Session-only model state:** State set through tools lasts one server
  process: session findings live in per-process
  `.planning/slugaudit/findings-<id>.db` files guarded by OS locks (swept by
  later processes once unlocked), and registered mechanics (`SessionMechanics`)
  plus the active profile (`session_profile`) live in process memory.
- **Human configuration:** Operator defaults live in read-only
  `~/.slugthug/slugaudit/config.json` and `SLUGAUDIT_*` environment variables.
  Tools never write them and model-facing text does not mention the global
  file. Limit precedence is profile baseline (`adaptive` by default, `lean`, or
  `audit`) -> `config.json` limit keys -> `SLUGAUDIT_*` overrides, with profile
  selection ordered `session_profile` > `SLUGAUDIT_PROFILE` > `config.json`.
- **Evidence-only surface and language scope:** Built-in `leads` count
  mechanical patterns only, with no ranking, severity, scoring, or checklists.
  Rust is the only tested language; other languages download Tree-sitter
  grammars on first use, remain available, and emit untested-language `notices`.
- **CLI and distribution surface:** One shipped binary (`slugaudit` with
  `serve`, `install`, `connect`, `disconnect`, `update`, `menu`, `version`, and
  `help`), `install.sh`, and directory metadata (`lhm.plugin.json`,
  `glama.json`).

## Shipped Milestones [built]

### 2026-11-25 — Startup Benchmark Framing and Planning Split [built]

- **Startup benchmark request framing:** Added the trailing `\n` delimiter to
  `INITIALIZE_REQUEST` (`benches/startup.rs:30`) so stdio JSON-RPC framing
  terminates the `initialize` request prior to `stdin` EOF.
- **Planning document split:** Migrated `.planning/PLAN.md` into
  `.planning/PROJECT_PLAN.md` (product scope, milestones, approved engineering
  work, and draft owner decisions) and `.planning/ARCHITECTURE.md`
  (architecture and module map), and removed `.planning/PLAN.md`.

### 2026-11-17 — Singleflight Sync, Result-Store Handle Cap, CLI/Release Gates, and rmcp 3.11 [built]

- `183a87d`:
  - **Per-project singleflight sync and read-first metadata check:**
    `SourceSyncManager` (`src/sync/manager.rs`,
    `src/sync/manager_orchestration.rs`) serializes concurrent MCP calls for the
    same project root in-process via per-project mutexes (`sync_locks`);
    `ensure_project_row` (`src/sync/manager_meta.rs`) checks `SELECT` before
    falling back to `INSERT OR IGNORE`, avoiding an unconditional SQLite write
    lock on every call; `SlugAuditServer::dispatch_health` (`src/server.rs`)
    runs `health` outside `blocking_ops` and reads database state inside one
    `Deferred` transaction (`src/tools/health.rs`).
  - **Result-store open-file cap and per-project store limits:**
    `Registry::plan_reclaim` (`src/store/result_store/registry.rs`) caps
    retained open result-store entries at `MAX_OPEN_RESULTS = 64` alongside
    `max_result_store_bytes`; `paginate` (`src/tools/paginate.rs`,
    `src/sync/result_stores.rs`) opens result stores with per-project resolved
    `ResultStoreLimits`; `resolve_limits` (`src/project/global_config.rs`)
    treats `max_result_store_bytes = 0` in `config.json` as unset when scaling
    from env overrides.
  - **CLI argument validation and release tag verification:** `cli::parse_args`
    (`src/cli.rs`) rejects extra arguments on `update`, `serve`, `help`, and
    `version`; `.github/workflows/release.yml` verifies that `v<Cargo.toml
    version>` equals `RELEASE_TAG` before building release binaries.
  - **Query SQL split, clone audit, and `rmcp` 3.11:** Extracted SQL
    validation/classification into `src/tools/query_sql.rs`; audited `.clone()`
    calls across `src/` with `// KEEP clone:` reasons (`PositionOverflow` now
    `Copy`); upgraded `rmcp` from `3.5.1` to `3.11.0` and updated
    `.planning/archive/mcp_surface.json` and `deny.toml`.
  - **Session/cache invalidation and SQLite indexing:** Added
    `MechanicScanCache::clear_project` on database rebuild and
    `project_control on`/`off`, and `clear_session_profile` on `off`; added
    `idx_files_path_hash`, `idx_files_language_path`, and
    `idx_evidence_call_callee` in `src/store/schema.sql`; switched
    `revision_files.rs` and `revision_edges.rs` to `RETURNING id` and
    `prepare_cached`.
  - **Structure, fuzz, and benchmark updates:** Collapsed `**` globstars,
    escaped literal `[` in `structure` file patterns, and honored explicit
    single-file `language` overrides (`src/tools/structure_source.rs`); added
    `FindingClearRequest` to `fuzz/fuzz_targets/request_arguments.rs`, `abort()`
    on fuzz fixture failure, and fuzz target compilation in
    `.github/workflows/quality.yml`; updated `benches/memory.rs` to remove the
    project database before each cold-sync iteration.

### 2026-11-03 — Guard Removal, Gates Separation, Zero-Panic Policy, and slugplan [built]

- `4f3d77f` / `94781c7` / `ec7e2cf`: `project_control on` delegates
  `.planning/` setup to `slugplan::ensure`; removed unreported-threat guards
  (`0600`/`0700` file modes, symlink refusals, `NOFOLLOW`); restricted capped
  reads to regular files (preventing FIFO hangs); stopped result-store
  operations from recreating `.planning/slugaudit/` after `off`; moved all
  `check_*` gate binaries and policy tests into dev-only crate `crates/gates`
  (`slugaudit-gates`).
- `77e6588` / `590b2fd` / `79e44f6` / `9f8593a`: Denied `unwrap_used`,
  `expect_used`, `panic`, `unreachable`, `indexing_slicing`,
  `arithmetic_side_effects`, and `assert*` (`disallowed_macros`) across
  workspace targets including tests (`slugaudit-test-check` `TestResult`);
  enforced `// KEEP saturating|wrapping:` annotations via
  `crates/gates/tests/saturating_policy.rs`; removed `let _ =`, `.ok()`,
  `map_or`, and `Err(_)` error-swallowing fallbacks.

### 2026-10-31 to 2026-11-02 — Two-Pass Sync, Session State, Contracts, and Listings [built]

- `cfaf1ce` / `04f3c8c` / `b1ddbb2` / `3805ca2` / `5763281`: Closed
  `project_control.status` enum, required `InvalidMechanic.index`/`id`, removed
  `report.parser_failure_files_total`; implemented two-pass sync (hash all
  files, sample changed files), result-store paging, shared input caps, and
  PRAGMA refusal before statement preparation.
- `263b63c` / `1111680` / `7fb6acf`: Established `.planning/slugaudit/` as the
  activation switch, moved findings/mechanics/profile to session-only storage,
  added `~/.slugthug/slugaudit/config.json`, and updated `lhm.plugin.json` to
  list all 10 tools (including `mechanic` and `finding_clear`).

### Recorded Gate and Benchmark Numbers [built]

- **Recorded gates (`77e6588` / `cfaf1ce`):** `cargo fmt --all --check` pass,
  `cargo clippy --workspace --all-targets -- -D warnings` pass (0 warnings),
  1,636 tests passed / 0 failed (at `77e6588`), all four `slugaudit-gates`
  binaries (`check_source_limits`, `check_no_duplicates`,
  `check_producer_coverage`, `check_mcp_surface`) pass, line coverage 91.5%
  (`79e44f6`) / 92.63% (`cfaf1ce`, gate threshold 83%).
- **Per-file single-thread warm-cache baseline (Ryzen 9 5900X, 32 GB, ZFS NVMe):**
  stat + open + read + BLAKE3 costs ~11 µs (<=4 KB), ~54 µs (64 KB), ~0.5 ms
  (1 MB), ~20 ms (32 MB); ~0.2 s per call at 10k files, ~1.9 s at 82k files.
- **Cold-sync peak RSS (`2026-11-25` audit measurements):** `61.5 MiB` on 10,000
  small Rust files, `193.4 MiB` on 10,000 dense Rust files, and
  `699.3–699.4 MiB` on 1,000 parser-heavy Rust files.

## Open Engineering Work [approved]

### Sync Working-Set Memory and Documentation Accuracy [approved]

- **Unbounded sync working-set RSS:** `ResourceLimits::max_total_import_bytes`
  (`src/model/limits.rs:99-105`, `src/model/profile.rs:135,162,185`: 2 GiB on
  `lean`, 32 GiB on `audit`, `max(avail_ram / 2, 4 GiB)` on `adaptive`) caps
  cumulative sampled file bytes, not process RSS. `build_upserts_and_deletions`
  (`src/sync/publish_diff.rs:197-237`, `upserts` loop `218-236`) retains sampled
  sources and normalized `Vec<FileRecord>` items across all changed files in
  memory before writing the revision transaction (reaching `699.3–699.4 MiB` RSS
  on 1,000 parser-heavy Rust files).
- **Performance documentation mismatch:** `docs/performance.md:51` states that
  in-memory file text during a call is capped by `max_total_import_bytes`,
  omitting that Tree-sitter ASTs, normalized evidence vectors, and SQLite
  buffers are not bounded by that setting.

### Rust Import Resolver and Source Span Constructors [approved]

- **Grouped `use` declarations:** `RustResolver::extract_reference`
  (`src/graph/resolve_rust.rs:41-56`) truncates at `{`, collapsing
  `use crate::{a, b}` to `"crate"` and emitting a single dependency edge to
  `src/lib.rs` (or `src/main.rs`) rather than resolving `a` and `b`.
- **`#[path]` test files:** `rust_module_dir` and `resolve`
  (`src/graph/resolve_rust.rs:99-109,193-212`) derive module hierarchy solely
  from filesystem paths without `#[path = "..."]` module attributes, so
  `use super::*` in a sibling `#[path = "foo_tests.rs"] mod tests;` file
  resolves to the parent directory module (`src/lib.rs` or `mod.rs`) instead of
  `foo.rs`.
- **Fallible `LineIndex::new`:** `LineIndex::new`
  (`src/model/span.rs:112,118-145,203-207`) is infallible and stores
  `Result<Vec<usize>, PositionOverflow>` inside `block_chars`, returning the
  stored error on every position lookup; change `LineIndex::new` to return
  `Result<Self, PositionOverflow>` and update callers
  (`src/evidence/normalize.rs`, `src/tools/structure_exec.rs`).

### Sync Metadata Error Propagation [approved]

- **Metadata `SELECT` error collapsed into `INSERT`:** `ensure_project_row`
  (`src/sync/manager_meta.rs:126-135`) uses
  `if let Ok(Some(row)) = connection.query_row(...).optional()`, routing any
  SQLite `SELECT` `Err` into the `else` branch (`INSERT OR IGNORE` + second
  `SELECT`) instead of propagating the read error and inserting only on
  `Ok(None)`.

### Limits, Caching, Paging, Error Contracts, and Gates [approved]

- **C5 slimming:** Delete the in-memory mechanic scan cache
  (`src/project/mechanic_scan_cache.rs`, `src/tools/mechanic_scan_cached.rs`);
  remove `check_performance` (`crates/gates/src/bin/check_performance/`) and
  the `criterion` dev-dependency (`Cargo.toml:45`) unless retained by owner
  decision; trim `GlobalConfig` / `ResourceLimits`
  (`src/project/global_config.rs`, `src/model/limits.rs`) to the knobs an
  operator needs.
- **Per-call sync wall-clock defaults:** Cut `max_sync_wall_clock` defaults in
  `src/model/profile.rs:145,172,204` from `120 s` (`lean`) and `1200 s`
  (`audit` / `adaptive`) to tens of seconds.
- **Cursor continuation disk-revision reporting:** `continue_page`
  (`src/tools/paginate.rs:305-388`) returns `page.revision_id` from when the
  result was stored; also report the current revision on disk at continuation
  time without failing the page.
- **Source-limit gate alignment:** `check_source_limits`
  (`crates/gates/src/bin/check_source_limits/main.rs:11-18`) still enforces the
  fixed `200 / 300 / 350` production code-line thresholds superseded by the
  `2026-11-02` `AGENTS.md` single-responsibility rule.
- **Error codes, remedies, and `structure` timeout behavior:** Add structured
  error codes and remedies to `ErrorData` sites that currently pass `None`;
  make `structure` timeout handling consistent (`src/tools/structure_exec.rs`:
  multi-file `parse_within` cancellation currently returns
  `Completeness::Partial` with earlier matches, whereas `out_of_budget`,
  single-file parse cancellation, and `QueryCursor` timeout discard matches and
  return `invalid_params`).
- **Config zero handling:** Unify handling of `0` between `config.json`
  (`src/project/global_config.rs:192-236`, `src/model/limit_overrides.rs:163`,
  where `0` logs a warning and is ignored) and `SLUGAUDIT_*` environment
  variables (`src/model/limits.rs:59-61`, where `0` fails startup).

### Test and Soak Suite Completeness [approved]

- **Test cleanup:** Remove obsolete tests covering deleted security guards,
  superseded designs, and wording-only checks.
- **Real-path sync invariant coverage:** Add real-path sync tests for
  edit/add/rename sequences, file-becomes-directory transitions, and FIFO paths
  to match existing deleted-DB, non-UTF-8 path, and deleted-file tests.
- **Large-repo soak incremental timing:** Extend `tests/large_repo_soak.rs`
  beyond the 10k-file cold import to pin timing for the unchanged-tree second
  call and the single-file-edit call.
- **Assertion failure prose:** Replace raw Rust expression strings in converted
  test `ensure*` messages (such as `src/evidence/mod_tests.rs:25` and
  `src/project/activation_disable_tests.rs:76`) with descriptive prose.
- **Strict JSON key checks in tests:** Replace `field()`
  (`src/evidence/mod_tests.rs:6-14`) and `is_none_or(Value::is_null)`
  (`tests/stdio_protocol.rs:624,733`) assertions where a missing JSON key passes.

## Open Owner Decisions and Budget Proposals [draft]

### Sync Working-Set Memory Budget Proposal [draft]

- Decide how to bound sync memory: add an explicit sync working-set budget
  (`lean: 256 MiB`, `audit: 1,024 MiB`,
  `adaptive: min(avail_ram / 4, 1,024 MiB)`) and/or flush parsed ASTs and
  normalized `FileRecord` / `evidence` rows to SQLite in bounded batches rather
  than holding all changed files' records in memory at once.

### Product, Gate, and Diagnostic Decisions [draft]

- **Fuzz target assertions:** Keep the 3 `assert!` / `assert_eq!` calls in
  `fuzz/fuzz_targets/` (`structure_query.rs:101,109` and `query_sql.rs:65`) so
  `libfuzzer` reports invariant violations by crashing?
- **Log file and line numbers:** Add `file:line` to log events in
  `init_logging` (`src/main.rs:262-281`)?
- **Performance gate and baseline file:** If `check_performance` is kept rather
  than removed under C5, compare the fastest of 5 runs so `discovery/walk_small`
  does not flake on host jitter, and decide whether untracked
  `.planning/archive/perf_baseline.json` stays local or is committed.
- **UTF-8 skip byte offset:** Include `Utf8Error::valid_up_to()` byte offset in
  `SampleError::NotUtf8` (`src/sync/sample.rs:50-57`, constructed at `128-134`),
  which formats `SkippedFile.reason` (while `SKIP_REASON_NOT_UTF8` `"not_utf8"`
  remains the fixed SQLite grouping code)?
- **Coverage gate:** Keep or drop `check_coverage` in CI?
- **Per-file size cap (`max_file_bytes`):** Keep `32 MiB` (`lean`) / `256 MiB+`
  (`audit` / `adaptive`), or lower to `4 MiB` (~10 ms read+hash ceiling on
  spinning disks)?
- **Per-call stored-text re-hash:** Keep re-hashing stored `files.content`
  against `files.content_hash` on every call, or change given it roughly doubles
  the per-call verification cost?
- **External MCP directory listings:** `lhm.plugin.json` in this repository
  already includes `mechanic` (all 10 tools), while published directory entries
  predate post-`v1.0.10` changes; audit what Glama, LHM, and the three other
  external directories currently state so every listing matches shipped code.

### Release and Client Rollout Timing [draft]

- **Local client reinstall and restart:** Deferred until the current open
  engineering work lands; any pre-change running server binary remains stale
  until reinstalled and restarted.
- **Version and release cut:** `Cargo.toml` and `lhm.plugin.json` specify
  `1.1.0` while the latest git tag is `v1.0.10`. Cut a new release tag only
  when the owner approves shipping, updating every directory listing the same
  day.

## Explicit Non-Goals (Not Doing) [approved]

- No file watcher.
- No security guards against unreported threats (no SQL authorizer, file
  permission policing, symlink/`NOFOLLOW` refusals, locks beyond session
  findings ownership, or network-filesystem refusals).
- No ranking, scoring, severity assignment, or checklists from built-in tools.
- No derived-row digest over `evidence` or `dependency_edges`.
- No removing non-Rust language support or the `install.sh` / `install` /
  `connect` / `disconnect` / `update` / `menu` surface, and no new non-Rust
  tests or fixes unless a user reports a bug.
- No exit-path wrapper layer around the MCP process (reply ledger, drain wait,
  or forced-exit grace).
- No new features, layers, or hardening until the Goal above is met and
  approved by the owner.
