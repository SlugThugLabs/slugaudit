# SlugAudit Architecture (`HEAD` `183a87d`)

Factual reference of SlugAudit's runtime layout, `src/` modules, and request/sync data flow at commit `183a87d`.

---

## 1. Where Each Part Runs (Runtime & Disk Layout)

### Process & Transport
- **Single shipped binary**: `slugaudit` (`Cargo.toml` package `slugaudit-mcp-rust`, binary entry `src/main.rs`, library root `src/lib.rs`).
- **Execution modes** (`src/main.rs`, `src/cli.rs::parse_args`):
  1. **MCP stdio server** (`slugaudit` with no arguments, `slugaudit serve`, or interactive `slugaudit menu` option `5`):
     - Runs on a multi-thread Tokio runtime (`#[tokio::main]`).
     - Validates `RUST_LOG` / `SLUGAUDIT_LOG_FORMAT` (`init_logging`) and `SLUGAUDIT_*` process limits (`model::init_process_limits`) before serving; invalid values exit `1` (CLI usage errors exit `2`).
     - `main.rs::serve_until_done` starts `SlugAuditServer::new().serve(rmcp::transport::stdio())` and races `running.waiting()` against `SIGTERM`/`SIGINT` (`tokio::signal`).
     - `rmcp` (`v3`, features `server`, `macros`, `transport-io`) handles JSON-RPC framing over stdin/stdout, `initialize`, session lifecycle, and transport shutdown. Stdout carries JSON-RPC only (`clippy::print_stdout` is denied workspace-wide); tracing logs write to stderr (`DEFAULT_LOG_FILTER = "warn,slugaudit_mcp_rust=info,rmcp=error"`, `SLUGAUDIT_LOG_FORMAT=text|json`).
  2. **Local CLI subcommands** (`src/cli.rs::Command`, output via `cli::out` / `cli::err`):
     - `serve` — start the MCP stdio server (default when no args are given).
     - `connect [agent]` — register `slugaudit` in a detected or named AI client (`src/connect.rs`, 23 client definitions in `src/connect_agents.rs::AGENTS`).
     - `disconnect [agent]` (alias `remove [agent]`) — remove the `slugaudit` registration from a client.
     - `install` — copy the running executable to `<slugaudit_dir>/slugaudit` (`0755`), retaining any prior binary as `slugaudit.prev`, and optionally append `<slugaudit_dir>` to `$SHELL` rc (`bash` `~/.bashrc`, `zsh` `~/.zshrc`, `fish` `~/.config/fish/config.fish`; `src/install.rs`, `src/install/path.rs`, `src/install_place.rs`). `install.sh` performs the corresponding shell-based release/cargo install to `~/.slugthug/slugaudit/slugaudit`.
     - `update` — fetch the latest GitHub release (`SlugThugLabs/slugaudit`, Linux `x86_64`) via `curl`, verify `SHA256SUMS` via `sha256sum`, retain `slugaudit.prev`, and atomically rename over the target binary (`src/update.rs`, `src/update_fetch.rs`).
     - `menu` — interactive terminal menu wrapping install, connect, disconnect, manual config snippet, and serve (`src/menu.rs`, `src/menu_render.rs`).
     - `version` (aliases `--version`, `-V`, `-v`) and `help` (aliases `--help`, `-h`, plus `<subcommand> --help|-h` via `Command::CommandHelp(Topic)` in `src/cli/usage.rs`).

### Global Human-Owned Paths & Environment
- **SlugAudit home directory** (`src/install.rs::slugthug_home`, `slugaudit_dir`): `$SLUGTHUG_HOME/slugaudit` if `SLUGTHUG_HOME` is non-empty, else `$HOME/.slugthug/slugaudit` (`~/.slugthug/slugaudit/`):
  - `~/.slugthug/slugaudit/slugaudit` — installed executable (preferred over `current_exe()` by `connect` and `update` when present).
  - `~/.slugthug/slugaudit/slugaudit.prev` — displaced executable retained by `install`, `update`, and `install.sh`.
  - `~/.slugthug/slugaudit/config.json` — optional human-owned defaults (`src/project/global_config.rs::GlobalConfig`, `#[serde(deny_unknown_fields)]`: `profile`, `max_file_bytes`, `max_parse_bytes`, `max_parse_wall_clock_ms`, `max_structure_query_bytes`, `max_total_import_bytes`, `default_page_bytes`, `max_result_store_bytes`, `result_active_grace_secs`). Read-only; MCP tools never write it and MCP instructions do not mention it.
- **Client config home** (`src/install.rs::config_home`): `$SLUGTHUG_CONFIG_HOME` if non-empty, else `$HOME` (base path used by `connect`/`disconnect` for client config files such as `~/.cursor/mcp.json`, `~/.claude.json`, `~/.gemini/settings.json`).
- **Environment variables** (read-only to tools):
  - Resource/limit overrides (`src/model/env_overrides.rs::EnvOverrides`, validated once into `OnceLock<EnvOverrides>`): `SLUGAUDIT_PROFILE` (`lean` | `audit` | `adaptive`), `SLUGAUDIT_MAX_FILE_BYTES`, `SLUGAUDIT_MAX_PARSE_BYTES`, `SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS`, `SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES`, `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES`, `SLUGAUDIT_DEFAULT_PAGE_BYTES`, `SLUGAUDIT_MAX_QUERY_SQL_BYTES`, `SLUGAUDIT_MAX_QUERY_VM_STEPS`, `SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS`, `SLUGAUDIT_MAX_QUERY_VALUE_BYTES`, `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES`, `SLUGAUDIT_MAX_STRUCTURE_MATCHES`, `SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS`, `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`, `SLUGAUDIT_MAX_RESULT_STORE_BYTES`, `SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS`.
  - Logging (`src/main.rs`): `RUST_LOG`, `SLUGAUDIT_LOG_FORMAT` (`json` | `text`).
  - Limit resolution precedence (`src/project/global_config.rs::resolve_limits`): base profile is chosen by `session_profile > SLUGAUDIT_PROFILE > config.json profile > adaptive`; limit overrides then apply in order `limits_for_profile(profile)` -> `config.json` limit keys -> `SLUGAUDIT_*` env limit variables.
- **Tree-sitter grammar cache**: `tree-sitter-language-pack` (`=1.18.0`, `src/parse/mod.rs::PACK_VERSION`) downloads non-bundled grammar shared libraries on first use into `<cache_base>/tree-sitter-language-pack/v1.18.0/libs` (where `<cache_base>` is `$TREE_SITTER_LANGUAGE_PACK_CACHE_DIR` or the OS user cache directory, e.g. `$XDG_CACHE_HOME` / `~/.cache` on Linux — outside `~/.slugthug/slugaudit/`).

### Per-Project Disk State (`<root>/.planning/slugaudit/`)
- **Activation switch** (`src/project/activation.rs`): a project is enabled iff `<root>/.planning/slugaudit/` exists as a directory (`find_project_root` walks `ancestors()` from the canonicalized request `path`).
  - `project_control` `action: "on"` (`src/tools/project_control.rs`, `project::enable`): calls `slugplan::ensure(root)` (creating `<root>/.planning/` and `<root>/.planning/README.md`), creates `<root>/.planning/slugaudit/`, appends `.planning/slugaudit/` to `<root>/.gitignore` via `project::gitignore::ensure_gitignore` unless already ignored (non-fatal), starts the process findings session, and builds/imports `project.db`.
  - `project_control` `action: "off"` (`project::disable`): drops this process's findings lock handle, removes `<root>/.planning/slugaudit/` (`remove_dir_all`, or `remove_file` if symlink), and removes `<root>/.planning/` via `remove_dir` only if empty. Never edits `.gitignore`.
- **Files inside `<root>/.planning/slugaudit/`**:
  - `project.db` (+ SQLite `-wal` and `-shm` sidecars): derived project index (`src/project/database_path.rs::DATABASE_FILENAME`, `PRAGMA user_version = 12` in `src/store/migrations.rs`). Disposable: deleted and rebuilt from disk when missing, corrupt, or mismatched on schema/contract/root metadata.
  - `findings-<id>.lock` and `findings-<id>.db` (+ `-wal`/`-shm` once written; `<id>` is a simple v4 UUID hex string): per-process session findings lock and SQLite database (`src/project/findings_session.rs`, `src/store/findings_db.rs`). On first touch of a project (`begin_process_session`), the process takes an exclusive OS file lock on `findings-<id>.lock` and sweeps unlocked `findings-*.db` / `findings-*.lock` files from dead processes plus any legacy `findings.db` (`src/project/findings_sweep.rs`). `findings-<id>.db` is created lazily on the first `finding` insert.
  - `results/` (Unix only; `src/store/result_store/`):
    - `results/<session>.lock` (`<session>` is hyphenated `util::session_id()` v4 UUID): advisory OS lock held for the lifetime of `ResultStore`; `owner::open_owner` sweeps dead owners' `.db` and `.lock` files on open, and `ResultStore::drop` unlinks its `.lock` file.
    - `results/<session>-<result>.db`: private per-result SQLite database created, opened (`journal_mode=off`, `locking_mode=exclusive`, `synchronous=OFF`), and **immediately unlinked** (`result_file::open_unlinked`), so rows live only on the open file descriptor until evicted from `Registry` or the process exits.

### In-Memory Session-Only State (Wiped on Process Start)
- **On `SlugAuditServer` / `SourceSyncManager`** (`src/server.rs`, `src/sync/manager.rs`):
  - `blocking_ops: Arc<Semaphore>` — caps concurrent blocking tool executions at `MAX_CONCURRENT_BLOCKING_OPS = 8` (`src/server_runner.rs`).
  - `session_mechanics: SessionMechanics` (`src/project/session_mechanics.rs`) — up to `MAX_SESSION_MECHANICS = 64` custom Tree-sitter mechanics (`Arc<Mutex<Vec<MechanicEntry>>>`) plus one-time `report_note_delivered: Arc<AtomicBool>`.
  - `mechanic_scan_cache: MechanicScanCache` (`src/project/mechanic_scan_cache.rs`) — FIFO-evicted scan cache (`DEFAULT_MECHANIC_CACHE_BYTES = 32 MiB`) keyed by `ScanKey { project, revision, glob, id, language, query, source }`; stale revisions of a project are dropped on lookup/insert/rebuild.
  - `result_stores: ResultStoreSlot` (`src/sync/result_stores.rs`) — single active `(PathBuf, Arc<ResultStore>)` slot; switching projects drops the prior `ResultStore` and invalidates prior cursors (each `ResultStore` also checks a per-instance random v4 `nonce: Uuid` on cursors).
  - `sync_locks: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>` — per-project singleflight mutexes (`183a87d`) keyed by canonical project root, serializing concurrent `ensure_current` calls on the same project in-process.
  - `last_sync_unix_seconds: Arc<AtomicI64>` and `last_sync_duration_ms: Arc<AtomicU64>`.
- **Process-global statics**:
  - `project::findings_session::SESSIONS` (`Mutex<Option<HashMap<PathBuf, Session>>>`) — active `findings-<id>.db` paths and held `File` lock handles per activation directory.
  - `project::session_profile::PROFILES` (`Mutex<Option<HashMap<PathBuf, AuditProfile>>>`) — per-project session profile set via `project_control on` (`profile`).
  - `parse::fetch_gate::LAST_FAILURE` (`Mutex<Option<Duration>>`) — process-wide `GRAMMAR_FETCH_COOLDOWN` (`10 min`) gate suppressing repeated grammar downloads after a network/cache failure.
  - `util::SESSION_ID` (`OnceLock<Uuid>`), `server_runner::NEXT_CALL_ID` (`AtomicU64`), `tools::tool_counters` (`AtomicU64` call/error/duration counters), `model::env_overrides::ENV` (`OnceLock<EnvOverrides>`), and the once-per-value limit-warning ledger (`model::limit_overrides`).

### Dev-Only Workspace Crates & Harnesses (Not Shipped)
- `crates/gates` (`slugaudit-gates`, `publish = false`): repository quality-gate binaries (`check_source_limits`, `check_no_duplicates`, `check_producer_coverage`, `check_mcp_surface`, `check_coverage`, `check_performance`, sharing `src/bin/gate_common/`) and policy tests (`crates/gates/tests/gate_binaries.rs`, `saturating_policy.rs`).
- `crates/test_check` (`slugaudit-test-check`, `publish = false`): panic-free `TestResult` assertion helpers (`Check`, `CheckErr`, `ensure`, `ensure_eq`, `ensure_ne`) used across tests.
- `fuzz/` (`slugaudit-mcp-rust-fuzz`, standalone `[workspace]` with its own `fuzz/Cargo.lock`, excluded from root `[workspace]`): `libfuzzer-sys` targets `request_arguments`, `structure_query`, and `query_sql`.
- `benches/` (`Cargo.toml` `[[bench]]`, `harness = false`): Criterion benchmarks `discovery`, `parsing`, `search`, `startup`, and `memory`.

---

## 2. The Parts (`src/` Modules & Responsibilities)

| Module / Path | Responsibility |
| :--- | :--- |
| `src/main.rs`, `src/cli.rs`, `src/cli/usage.rs` | Binary entrypoint, `args_os` parsing into `cli::Command`, stderr tracing initialization, process limit check, signal handling, and pipe-safe stdout/stderr writers (`cli::out`, `cli::err`). |
| `src/install.rs`, `src/install/path.rs`, `src/install_place.rs` | Resolution of `slugthug_home`, `slugaudit_dir`, `config_home`, and `running_binary`; staged binary copy, `slugaudit.prev` retention, atomic rename, and idempotent `$SHELL` `PATH` append. |
| `src/connect.rs`, `src/connect/*.rs`, `src/connect_*.rs` | Client MCP registration/removal: static 23-agent catalog (`connect_agents.rs`), CLI dialect execution with timeout (`connect_cli.rs`, `connect_exec.rs`, `connect_run.rs`), atomic JSON config merging (`connect/config_file.rs`, `config_atomic.rs`), interactive picker (`connect/interactive.rs`), and manual refusal remedies (`connect_guidance.rs`). |
| `src/update.rs`, `src/update_fetch.rs` | Self-update against GitHub releases (`curl` fetch, version comparison, `sha256sum` verification, atomic binary replacement via `install_place`). |
| `src/menu.rs`, `src/menu_render.rs` | Interactive 6-option setup menu and terminal rendering. |
| `src/server.rs`, `src/server/handlers.rs`, `src/server_runner.rs`, `src/server_progress.rs`, `src/resources.rs` | `SlugAuditServer` (`rmcp::ServerHandler`), `INSTRUCTIONS`, `#[tool_router]` registration of the 10 MCP tools (`handlers.rs`), 8-permit `spawn_blocking` execution + per-call tracing (`server_runner.rs`), throttled `/notifications/progress` adapter (`server_progress.rs`), and compile-time embedded `slugaudit://` resources (`resources.rs`). |
| `src/tools/` | Tool request/response contracts and orchestration: shared sync/read/paging helpers (`context.rs`, `paginate.rs`, `page.rs`, `page_size.rs`, `tool_limits.rs`, `input_caps.rs`, `tool_counters.rs`, `language_notices.rs`, `sql_predicates.rs`) and tool implementations (`report*.rs`, `leads*.rs`, `mechanic*.rs`, `query*.rs`, `structure*.rs`, `finding*.rs`, `project_control.rs`, `health*.rs`). |
| `src/sync/` | `SourceSyncManager` (`manager.rs`, `manager_orchestration.rs` — including `verify_metadata_or_rebuild`, `manager_meta.rs` — including `ensure_project_row` and `ProjectMetaError::Corrupt`), `ResultStoreSlot` (`result_stores.rs`), filesystem walk (`discovery.rs`), bounded worker pool (`file_pool.rs`), BLAKE3 file/manifest hashing (`hash.rs`, `hash_io.rs`, `hash_batch.rs`), stored-content verification (`verify_stored.rs`), diffing (`manifest.rs`, `publish_diff.rs`), bounded sampling (`sample.rs`, `sample_batch.rs`, `publish_sample.rs`), Tree-sitter analysis glue & evidence caps (`analyze*.rs`, `evidence_limits.rs`), path classification (`classify.rs`), pre-commit TOCTOU re-sampling (`revalidate.rs`), and CAS revision transactions (`publish.rs`, `publish_cas.rs`, `publish_attempt.rs`, `revision*.rs`, including `revision_edges.rs` for dependency edge resolution and incremental Rust dependent re-resolution). |
| `src/store/` | SQLite connection setup (`connection.rs`, `connection_config.rs` — WAL, `foreign_keys = ON`, 5s `busy_timeout`), `PRAGMA user_version` schema initialization (`migrations.rs`, `schema.sql`), custom SQL functions (`sql_functions.rs`), session findings DB (`findings_db.rs`, `findings_schema.sql`), and unlinked SQLite paged-result store (`result_store/`). |
| `src/parse/` | Language detection and prose classification (`language.rs`; `TESTED_LANGUAGES = &["rust"]`, `PROSE_LANGUAGE = "prose"`), deadline-bounded `tree-sitter` parser wrapper (`budget.rs`), `PACK_VERSION` (`mod.rs`), and 10-minute grammar download cooldown gate (`fetch_gate.rs`). |
| `src/evidence/` | Neutral evidence extraction (`normalize.rs`, `normalize_builders.rs`) turning `tree_sitter_language_pack::process` output plus iterative heap-stack AST walkers (`MAX_WALK_DEPTH = 512`) for calls/Rust macro calls (`calls.rs`, `calls/*`), local bindings (`bindings.rs`), generic imports (`generic_imports.rs`), and Rust `#[path = "..."]` / `#[cfg_attr(..., path = "...")]` module attributes (`rust_mod_paths.rs::enrich_rust_mod_paths`, `rust_path_attr.rs::parse_attribute_item`, attaching `rust_mod_path` payloads to external Rust `Structure` `Module` items) into SQL-ready `EvidenceRow`s (`sql.rs`). |
| `src/graph/` | Import-source to project-path resolution (`mod.rs::resolve_imports_with_rust_parents`, `resolve.rs`, `reference.rs`, `resolve_rust.rs`, `rust_paths.rs`, `rust_use_tree.rs::parse_rust_import` for multi-target grouped `use` expansion, `rust_mod_parents.rs` for `#[path]` `super::` parent walks, and per-language resolvers in `resolver/`, with `resolver/registry.rs::resolve_many` deduplicating edges per statement) producing `ResolvedEdge`s (`Resolved`, `External`, `Unresolved`). |
| `src/project/` | Canonical `ProjectRoot` (`root.rs`), `.planning/slugaudit` enable/disable & root lookup (`activation.rs`, `database_path.rs`), `.gitignore` guard (`gitignore.rs`), `GlobalConfig` & limit resolution (`global_config.rs`), `findings_session.rs` & `findings_sweep.rs`, `session_mechanics.rs` (`mechanic_entry.rs`), `mechanic_scan_cache.rs`, and `session_profile.rs`. |
| `src/ignore_rules.rs` | `ignore::WalkBuilder` setup (`standard_filters(true)`, `parents(false)`, `follow_links(false)`, `hidden(false)`) and hardcoded exclusions (`.git`, `.planning/slugaudit`, and scratch filenames `*.tmp`, `*.bak`, `*.swp`, `*~`, `scratch.*`, `*.claude_output.txt`). *(Note: `src/ignore_rules/build.rs` exists on disk as an unreferenced file without a `mod build` declaration).* |
| `src/model/` | Shared data types and resource bounds: `EvidenceItem` / `EvidenceKind` (`evidence.rs`), 1-based `Span` / fallible `LineIndex::new` (`span.rs`, returning `Result<Self, PositionOverflow>`), `SourceIdentity` (`source.rs`), `ParserRun` enums (`parser.rs`), `AuditProfile` (`profile.rs`), `ResourceLimits` / `EvidenceLimits` (`limits.rs`, `limit_overrides.rs`), `EnvOverrides` (`env_overrides.rs`, `env_error.rs`), and host RAM detection (`system_ram.rs`). |
| Shared support | `src/error_codes.rs` (closed `ErrorCode` wire enum + static `remedy` payloads), `src/clock.rs` (checked Unix-epoch and millisecond conversions), `src/progress.rs` (`ProgressEvent` / `ProgressSink`), `src/probe.rs` (`which` PATH lookup for `connect`), `src/prompt.rs` (256-byte capped stdin prompt reader), `src/util.rs` + `src/util/{capped_read.rs,rust_comments.rs,style.rs,time.rs}` (regular-file capped reads, Rust comment lexing, `Deadline`, mutex `lock` poison handling, BLAKE3 log redaction, session UUID, JSON byte counter). |

---

## 3. How They Talk to Each Other (Request & Sync Data Flow)

### MCP Tool Call Flow
1. **Transport & Router**: JSON-RPC requests arrive on stdin via `rmcp` -> `SlugAuditServer` (`src/server.rs`). `resources/list` and `resources/read` serve compile-time embedded `CATALOG` entries (`src/resources.rs`, URI capped at `8 KiB`) with no disk or DB access. `tools/call` routes through `#[tool_router]` in `src/server/handlers.rs`:
2. **`health` (`SlugAuditServer::dispatch_health`)**:
   - Runs directly on `tokio::task::spawn_blocking` **without** acquiring a `blocking_ops` semaphore permit.
   - Calls `tools::health` (`src/tools/health.rs`): never syncs and never mutates. When `path` is supplied, opens `project.db` read-only (`store::open_existing`) in a `DEFERRED` transaction to report `phase`, `remedy`, `database`, and optional `verbose` `detail`.
3. **All Other 9 Tools (`SlugAuditServer::dispatch` -> `server_runner::run_blocking`)**:
   - Enters a `tool_call` tracing span (`call_id`, `session_id`), emits `0.0` progress, acquires 1 of 8 `blocking_ops` semaphore permits, emits `0.5` progress, runs the tool closure on `spawn_blocking`, emits `1.0` progress, bumps `ToolCounters`, and logs one completion event.
   - **Paged continuations (`cursor` supplied)** on `query`, `leads`, `structure`, `finding_read`, and `mechanic` (`list` only):
     - `tools::paginate::page_request` selects `PageRequest::Cursor(token)` -> `paginate::continue_page` (`src/tools/paginate.rs`).
     - Decodes `(nonce, result_id, ord)` against the open `ResultStore` in `manager.result_stores()`, verifies `meta.tool` matches the calling tool, and reads the next byte-bounded page from the unlinked result SQLite file at the original `revision_id` — **no sync, no `project.db` open, `path` ignored**.
   - **`project_control` (`src/tools/project_control.rs`)**:
     - `action: "on"`: resolves `ProjectRoot::resolve`, creates `.planning/` + `README.md` (`slugplan::ensure`) and `.planning/slugaudit/`, updates `.gitignore`, starts `findings_session`, sets optional `session_profile`, clears `mechanic_scan_cache` and `ResultStoreSlot` for that runtime dir, opens/creates `project.db` (`store::open_read_write`, discarding/recreating on corruption), and calls `sync::publish` directly.
     - `action: "off"`: clears `mechanic_scan_cache` and `session_profile`, and calls `project::disable` to release the findings lock and delete `.planning/slugaudit/` (plus `.planning/` if empty).
   - **`mechanic` with `action: "remove"` (`src/tools/mechanic.rs`, `mechanic_register.rs`)**:
     - Resolves the active project root and limits, removes `id` from `MechanicScanCache` and `SessionMechanics`, and returns the updated catalogue without running `ensure_current`.
   - **Active-project fresh calls (`report`, `leads`, `mechanic` `add`/`list`, `structure`, `query`, `finding`, `finding_read`, `finding_clear`)**:
     - Call `tools::context::ensure_synced` -> `SourceSyncManager::ensure_current(path, sink)`.
     - Read index state inside `tools::context::with_verified_read` (opens `project.db` read-only, starts `BEGIN DEFERRED`, verifies `current_revision_id == synced.revision_id` or fails with `stale_revision`).
     - Paged tools (`query`, `leads`, `structure`, `finding_read`, `mechanic list`) stream rows through `RowSink` into a new unlinked `ResultStore` SQLite file (`tools::paginate::paginate`) and serve page 1 + `page.cursor`.
     - `finding` and `finding_clear` write to this process's `findings-<id>.db` via `tools::finding_store::with_findings_write` (`BEGIN IMMEDIATE` on `store::open_findings`).

### Every-Call Sync Pipeline (`SourceSyncManager::ensure_current`)
1. **Root Resolution, Singleflight Lock & Session Init** (`src/sync/manager_orchestration.rs`):
   - `project::resolve_project(path)` locates the nearest ancestor with `<root>/.planning/slugaudit/` (else `ErrorCode::ProjectNotEnabled`) and `<root>/.planning/slugaudit/project.db`.
   - Acquires the per-project `Arc<Mutex<()>>` from `SourceSyncManager::sync_locks` (`manager.rs::project_sync_lock`), serializing concurrent syncs of the same repo in-process.
   - Calls `project::begin_process_session(&root)` to lock `findings-<id>.lock` and sweep dead sessions' findings files on first touch.
2. **Database Open, Metadata Check & Auto-Rebuild** (`open_or_rebuild`, `verify_metadata_or_rebuild`, `src/sync/manager_meta.rs`):
   - Opens `project.db` via `store::open_existing_read_write` (enables WAL + `foreign_keys = ON` + 5s `busy_timeout`, registers SQL functions, checks `PRAGMA user_version == 12`).
   - `ensure_project_row` runs `SELECT` first on `project` row `id = 1` (`root_path`, `contract_version = 1`, `schema_version = 1`), propagating read errors via `map_metadata_sql_error` (`ProjectMetaError::Corrupt` for SQLite corruption, `ProjectMetaError::Other` otherwise) and inserting (`INSERT OR IGNORE`) + verifying only on `Ok(None)`.
   - In `open_or_rebuild`, `verify_metadata_or_rebuild`, and `publish_or_rebuild`, if `project.db` is missing, corrupt (`DatabaseCorrupt`, `NotADatabase`, `UnsupportedVersion`, `ProjectMetaError::Corrupt`, or mid-publish corruption), or stale (`RootMismatch`, `IncompatibleVersion`), `rebuild_and_finish` unlinks `project.db` + `-wal`/`-shm` (`store::discard_corrupt_database`), clears `MechanicScanCache` for that DB, recreates and publishes from scratch (`publish_from_scratch`), and attaches `DatabaseDiscard` when an existing bad DB was discarded; `ProjectMetaError::Other` fails the call.
3. **Bounded CAS Retry Loop** (`src/sync/publish_cas.rs::publish` -> `publish_with_limits`):
   - Computes `project::effective_limits(root)` and creates one shared `Deadline` (`max_sync_wall_clock`) across up to `MAX_CAS_RETRIES = 4` retries on `RevisionError::StaleBaseline` or `PublishError::ChangedDuringSample`.
4. **Pass 1 — Walk, Stream-Hash, Diff & Tamper Check** (`src/sync/publish_attempt.rs::hash_and_diff`; no file text held in RAM):
   - **Discovery** (`ignore_rules::indexable_walker` -> `sync/discovery.rs::discover_with_deadline`): walks `root` pruning `.git` and `.planning/slugaudit`, skipping scratch filenames, sniffing 8,000 bytes for NUL (`Indexed` vs `Binary`), and recording unreadable or non-UTF-8 paths.
   - **Parallel BLAKE3 stream-hashing** (`sync/file_pool.rs` + `sync/hash_batch.rs::hash_all_with_deadline` + `sync/hash_io.rs`): worker pool (`min(files, available_parallelism, 8)`) hashes regular files in 64 KiB chunks (`hash_algorithm = 'blake3-v1'`); files exceeding `max_file_bytes` become `skipped` (`skip_reason = 'oversize'`) with stat-derived BLAKE3 identity (`hash_algorithm = 'stat-identity'`).
   - **Manifest diff** (`sync/publish_diff.rs::diff_against_stored` + `sync/manifest.rs::compare`): diffs current `(path, content_hash)` against `SELECT path, content_hash FROM files`, adds `LoadFailed` retry paths if `parse::fetch_gate::load_failed_retry_due()` is open (`publish_diff/load_failed.rs`), and computes `aggregate_manifest_hash`.
   - **Stored-content tamper re-hash** (`sync/verify_stored.rs::content_mismatches` -> `force_content_repairs`): streams `SELECT path, content, content_hash FROM files WHERE content IS NOT NULL`, recomputes BLAKE3 over `content`, and forces any mismatched path still on disk into `diff.changes` as `Modified`.
   - Syncs `unrepresentable_paths`. **No-change fast path**: if `diff.changes` is empty, `parser_pack_version == PACK_VERSION` (`"1.18.0"`), and a current revision exists, `unchanged_report` re-checks `current_revision` and returns immediately without sampling or writing a new revision row.
5. **Pass 2 — Selective Sampling, Parsing, TOCTOU Revalidation & Atomic Commit** (`try_publish` -> `commit_attempt`):
   - **Sampling** (`sync/publish_sample.rs`, `sync/sample_batch.rs`, `sync/sample.rs`): reads only added/modified files (or all files when `parser_pack_version` changed) on `file_pool` via `util::read_file_capped_bytes`, enforcing regular-file checks, `max_file_bytes`, cumulative `max_total_import_bytes`, and UTF-8 validity; `check_samples_match_pass1` aborts with `ChangedDuringSample` if any sampled hash differs from Pass 1.
   - **Analysis & Evidence Extraction** (`publish_diff::build_upserts_and_deletions` -> `sample::to_file_record` -> `sync/analyze.rs::analyze`):
     - `parse::detect_language` marks prose/ignore-list files as `language = 'prose'` (`ContentOnly`, unparsed) and maps code/data paths via `tree-sitter-language-pack`.
     - Within `max_parse_bytes` and `parse::fetch_gate`, runs `evidence::extract_within` (`src/evidence/`) under `min(max_parse_wall_clock_ms, sync_deadline)` to extract `Structure`, `Import`, `Export`, `Comment`, `Docstring`, `Symbol`, `Diagnostic`, and `Call` items, then caps per-file counts/bytes (`sync/evidence_limits.rs`).
   - **Pre-commit TOCTOU revalidation** (`sync/revalidate.rs::revalidate_unchanged_since_sample`): re-samples every file in `upserts` and aborts with `ChangedDuringSample` if any file changed on disk during analysis.
   - **Atomic CAS publish transaction** (`sync/revision.rs::publish_revision` in `BEGIN IMMEDIATE`):
     1. `assert_baseline` verifies `revisions WHERE is_current = 1` still matches `expected_current` (else `StaleBaseline`).
     2. Inserts `revisions` row (`rev-<id>`, `is_current = 0`).
     3. Nulls inbound `dependency_edges` (`to_file_id = NULL, resolution_kind = 'Unresolved'`) for deleted targets and deletes removed `files` rows (`ON DELETE CASCADE` removes their `evidence` and outbound `dependency_edges`).
     4. Upserts `files` (with path class from `sync/classify.rs`) + replaces `evidence` (`revision_files.rs`), and upserts `file_kind = 'skipped'` rows (`revision_skipped.rs`).
     5. Resolves `Import` evidence via `graph::resolve_imports_with_rust_parents` (`src/graph/`) into `dependency_edges` (`revision_edges.rs::resolve_and_store` and `re_resolve_unresolved`), loading `rust_mod_path` parent mappings (`load_rust_module_parents` -> `rust_mod_parents::RustModuleParents`) and incrementally re-resolving affected `#[path]` child files and unchanged Rust dependents with `super::` or `Low`-confidence edges (`re_resolve_rust_dependents`, `collect_rust_dependents`), as well as prior `Unresolved` (`syntax_unmodeled = 0`) edges when files were added or deleted.
     6. Flips `revisions.is_current` to the new row and commits.

### SQLite Schema & Retrieval
- **`project.db` tables** (`src/store/schema.sql`, `PRAGMA user_version = 12`):
  - `project`: singleton metadata row (`id = 1`, `project_id`, `root_path`, `contract_version`, `schema_version`, `created_at_unix`).
  - `revisions`: append-only revision log (`id`, `revision_id`, `manifest_hash`, `parser_pack_version`, `created_at_unix`, `is_current`; partial unique index `idx_revisions_single_current` on `is_current WHERE is_current = 1`).
  - `unrepresentable_paths`: `path BLOB PRIMARY KEY` (raw bytes of discovered non-UTF-8 project-relative paths).
  - `files`: one row per representable discovered file (`id`, `path`, `file_kind` IN `'indexed'|'binary'|'skipped'`, `skip_reason` IN `'oversize'|'not_utf8'|'unreadable'|'stat_failed'` or NULL, `file_class` IN `'authored'|'generated'|'vendored'`, `content TEXT` holding full UTF-8 text for `'indexed'` rows, `content_hash`, `hash_algorithm` `'blake3-v1'|'stat-identity'`, `byte_len`, `modified_unix_seconds`, `language`, `language_detected`, `parser_availability`, `parse_outcome`, `parse_error_reason`, `extraction_completeness`, `last_revision_id`).
  - `evidence`: normalized AST/diagnostic facts (`id`, `file_id` FK -> `files(id) ON DELETE CASCADE`, `key`, `kind` IN `'Structure'|'Import'|'Export'|'Comment'|'Docstring'|'Symbol'|'Diagnostic'|'Call'`, `origin`, `span_availability`, `start_byte`, `end_byte`, `start_line`, `start_column`, `end_line`, `end_column`, `payload TEXT` JSON; `UNIQUE(file_id, key)`, indexed by `kind`, `file_id`, and `json_extract(payload, '$.callee_name'), file_id WHERE kind = 'Call'`).
  - `dependency_edges`: import graph (`id`, `from_file_id` FK `ON DELETE CASCADE`, `to_file_id` nullable FK `ON DELETE CASCADE`, `raw_import_text`, `resolution_kind` IN `'Resolved'|'Unresolved'|'External'`, `confidence` IN `'High'|'Low'` or NULL, `syntax_unmodeled`).
- **Session SQLite schemas**:
  - `findings-<id>.db` (`src/store/findings_schema.sql`): `findings` (`id`, `path`, `source_hash`, `line_start`, `line_end`, `severity`, `category`, `title`, `description`, `created_at_unix`, `evidence_revision`).
  - Unlinked `results/<session>-<result>.db` (`src/store/result_store/result_file.rs`): `meta (key TEXT PRIMARY KEY, value)` and `rows (ord INTEGER PRIMARY KEY, nonfinite TEXT, c0, c1, ...)`.
- **Custom SQLite scalar functions & `query` execution**:
  - Registered on every `project.db` connection by `src/store/sql_functions.rs::register` (invoked from `store::connection::open_read_write_with` and `open_existing`, with a 32 MiB `MAX_LINE_FUNCTION_INPUT_BYTES` cap; `#[cfg(test)]` additionally registers `golden_nan()`):
    - `line_count(text)` — 1-based `\n`-separated line count (`0` for `""`, `NULL` -> `NULL`).
    - `line_range(text, start, end)` — 1-based inclusive line slice `start..=end` without the final line terminator (`\n` or `\r\n`).
  - `src/tools/query_sql.rs` and `src/tools/query_exec.rs` validate SQL by rejecting any `PRAGMA` / `EXPLAIN PRAGMA` / `EXPLAIN QUERY PLAN PRAGMA` before preparation (`any_statement_is_pragma`), preparing one `readonly()` statement with unique column names, wrapping it as `SELECT * FROM (\n{body}\n) LIMIT -1 OFFSET 0`, setting `SQLITE_LIMIT_LENGTH` and `SQLITE_LIMIT_SQL_LENGTH`, and installing a 1,000-step `progress_handler` enforcing `max_query_vm_steps` and `max_query_wall_clock`.
