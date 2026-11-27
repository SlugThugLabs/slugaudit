# SlugAudit Architecture

Factual reference of SlugAudit's runtime layout, `src/` modules, trust boundary, and request/sync data flow.

---

## 1. Where Each Part Runs (Runtime, Trust Boundary & Disk Layout)

### Trust Boundary
- **Host-harness model**: SlugAudit is a local developer tool running inside the host coding harness's workspace permission boundary. The host harness and OS own file permissions and already grant the model direct access to repository files.
- **Disposable derived scratch**: `<root>/.planning/slugaudit/` (`project.db`, `findings-<id>.db`, `results/`) holds disposable scratch data derived from that repository using normal user file modes (no `0600` chmod policing, no SQL authorizers, no symlink/path jails).
- **Bounded execution**: SlugAudit bounds accidental runaway calls and parses (read-only SQLite connection and single-`SELECT`/`WITH` wrapper on `query`, SQLite VM-step/wall-clock/value-byte caps, Tree-sitter parse deadlines and match caps, capped regular-file reads, and bounded child-process timeouts).

### Process & Transport
- **Single shipped binary**: `slugaudit` (`Cargo.toml` package `slugaudit-mcp-rust`, binary entry `src/main.rs`, library root `src/lib.rs`).
- **Static Rust grammar** (`.cargo/config.toml` and `install.sh` `--from-source`): `TSLP_LINK_MODE = "static"` and `TSLP_LANGUAGES = "rust"` compile `tree-sitter-rust` statically into `slugaudit` (`tree_sitter_language_pack::has_parser("rust") == true` offline with zero runtime download). Other languages download shared grammar libraries on first use.
- **Execution modes** (`src/main.rs`, `src/cli.rs::parse_args`):
  1. **MCP stdio server** (`slugaudit` with no arguments, `slugaudit serve`, or interactive `slugaudit menu` option `5`):
     - Runs on a multi-thread Tokio runtime (`#[tokio::main]`).
     - Validates `RUST_LOG` / `SLUGAUDIT_LOG_FORMAT` (`init_logging`) and startup configuration (`project::validate_startup_config` across `~/.slugthug/slugaudit/config.json`, `SLUGAUDIT_*` env vars, and host memory when `Adaptive`) before serving; invalid values exit `1` (CLI usage errors exit `2`).
     - `main.rs::serve_until_done` starts `SlugAuditServer::new().serve(rmcp::transport::stdio())` and races `running.waiting()` against `SIGTERM`/`SIGINT` (`tokio::signal`).
     - `rmcp` (`v3.11.0`, features `server`, `macros`, `transport-io`) handles JSON-RPC framing over stdin/stdout, `initialize`, session lifecycle, and transport shutdown. Stdout carries JSON-RPC only (`clippy::print_stdout` is denied workspace-wide); tracing logs write to stderr (`DEFAULT_LOG_FILTER = "warn,slugaudit_mcp_rust=info,rmcp=error"`, `SLUGAUDIT_LOG_FORMAT=text|json`).
  2. **Local CLI subcommands** (`src/cli.rs::Command`, output via `cli::out` / `cli::err`):
     - `serve` — start the MCP stdio server (default when no args are given).
     - `connect [agent]` — register `slugaudit` in a detected or named AI client (`src/connect/mod.rs`, 23 client definitions in `src/connect/agents.rs::AGENTS`, quote/escape-aware `.jsonc` handling in `src/connect/jsonc.rs` that refuses overwriting commented `.jsonc` configs via `ConnectError::CommentedConfig`, and `probe::cli_installed` verification so an unrelated `/usr/bin/q` is never invoked as Amazon Q CLI).
     - `disconnect [agent]` (alias `remove [agent]`) — remove the `slugaudit` registration from a client (`src/connect/exec.rs`, `src/connect/interactive.rs`).
     - `install` — copy the running executable via `create_new(true)` staging to `<slugaudit_dir>/slugaudit` (`0755`), retaining any prior binary as `slugaudit.prev`, and optionally append `<slugaudit_dir>` to `$SHELL` rc (`src/install/mod.rs`, `src/install/path.rs`, `src/install/place.rs`). `install.sh` performs the shell-based pinned-release (or `--from-source` with `TSLP_LINK_MODE=static TSLP_LANGUAGES=rust`) install to `~/.slugthug/slugaudit/slugaudit`.
     - `update` — fetch the latest GitHub release (`SlugThugLabs/slugaudit`, Linux `x86_64`) via bounded `curl -q`, compare SemVer versions, verify `SHA256SUMS` via bounded `sha256sum`, retain `slugaudit.prev`, and atomically rename over the target binary (`src/update.rs`, `src/update_fetch.rs`, `src/update_proc.rs`, `src/update_verify.rs`).
     - `menu` — interactive 6-option terminal menu wrapping install, connect, disconnect, manual config snippet (`connect::other_agent_instructions`), and serve (`src/menu.rs`).
     - `version` (aliases `--version`, `-V`, `-v`) and `help` (aliases `--help`, `-h`, plus `<subcommand> --help|-h` via `Command::CommandHelp(Topic)` in `src/cli/usage.rs`).

### Global Human-Owned Paths & Environment
- **SlugAudit home directory** (`src/install/mod.rs::slugthug_home`, `slugaudit_dir`): `$SLUGTHUG_HOME/slugaudit` if `SLUGTHUG_HOME` is non-empty, else `$HOME/.slugthug/slugaudit` (`~/.slugthug/slugaudit/`):
  - `~/.slugthug/slugaudit/slugaudit` — installed executable (preferred over `current_exe()` by `connect` and `update` when present).
  - `~/.slugthug/slugaudit/slugaudit.prev` — displaced executable retained by `install`, `update`, and `install.sh`.
  - `~/.slugthug/slugaudit/config.json` — optional human-owned defaults (`src/project/global_config.rs::GlobalConfig = LimitOverrides`, `#[serde(deny_unknown_fields)]`: `profile` plus all 16 `ResourceLimits` limit keys; zero values fail closed with `ConfigError::Zero`). Read-only; MCP tools never write it and MCP instructions do not mention it.
- **Client config home** (`src/install/mod.rs::config_home`): `$SLUGTHUG_CONFIG_HOME` if non-empty, else `$HOME`.
- **Environment variables** (read-only to tools):
  - Resource/limit overrides (`src/model/env_overrides.rs::EnvOverrides`): `SLUGAUDIT_PROFILE` (`lean` | `audit` | `adaptive`) plus the 16 `SLUGAUDIT_MAX_*` / `SLUGAUDIT_DEFAULT_PAGE_BYTES` / `SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS` limit variables.
  - Logging (`src/main.rs`): `RUST_LOG`, `SLUGAUDIT_LOG_FORMAT` (`json` | `text`).
  - Precedence (`src/project/limits.rs`): base profile is chosen by `session_profile > SLUGAUDIT_PROFILE > config.json profile > adaptive`; limit overrides then apply in order `limits_for_profile(profile)` -> `config.json` limit keys -> `SLUGAUDIT_*` env limit variables.
- **Tree-sitter grammar cache**: `tree-sitter-language-pack` (`=1.18.0`, `src/parse/mod.rs::PACK_VERSION`) downloads non-Rust shared grammar libraries on first use into `<cache_base>/tree-sitter-language-pack/v1.18.0/libs`.

### Per-Project Disk State (`<root>/.planning/slugaudit/`)
- **Activation switch** (`src/project/activation.rs`): a project is enabled iff `<root>/.planning/slugaudit/` exists as a directory.
  - `project_control` `action: "on"` calls `slugplan::ensure(root)`, creates `<root>/.planning/slugaudit/`, appends `.planning/slugaudit/` to `<root>/.gitignore` unless already ignored, starts the process findings session, and runs the initial import.
  - `project_control` `action: "off"` releases open `ResultStore` handles and session state for that project, drops this process's findings lock handle, removes `<root>/.planning/slugaudit/`, and removes `<root>/.planning/` only if empty.
- **Files inside `<root>/.planning/slugaudit/`**:
  - `project.db` (+ `-wal` / `-shm`): derived project index (`PRAGMA user_version = 13` in `src/store/migrations.rs`). Deleted and rebuilt when missing, corrupt, or mismatched on schema/contract/root metadata.
  - `findings-<id>.lock` and `findings-<id>.db`: per-process session findings lock and SQLite database (`src/project/findings_session.rs`, `src/store/findings_db.rs`). Unlocked files from dead processes are swept on first touch.
  - `results/<session>.lock` and private `results/<session>-<result>.db`: paged-result SQLite spill files (`src/store/result_store/`), unlinked immediately after open on Unix (or cleaned on eviction/drop on non-Unix) and swept across dead sessions.

### In-Memory Session-Only State (Wiped on Process Start)
- **On `SlugAuditServer` / `SourceSyncManager`** (`src/server.rs`, `src/sync/manager.rs`):
  - `blocking_ops: Arc<Semaphore>` — caps concurrent blocking tool executions at `MAX_CONCURRENT_BLOCKING_OPS = 8`.
  - `session_mechanics: SessionMechanics` (`src/project/session_mechanics.rs`) — per-project session Tree-sitter mechanics (`MAX_SESSION_MECHANICS = 64`, validated at `mechanic add` time; no persistent or invalid-entry state and no separate mechanic scan cache) plus one-time `report_note_delivered`.
  - `result_stores: ResultStoreSlot` (`src/sync/result_stores.rs`) — single-project active `ResultStore` slot routing cursor continuations (`store_for_cursor`) and per-project `ResultStoreLimits`.
  - `sync_locks: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>` — per-project singleflight mutexes serializing concurrent syncs of the same project in-process.
  - `last_sync_unix_seconds: Arc<AtomicI64>` and `last_sync_duration_ms: Arc<AtomicU64>`.
- **Process-global & thread-local caches/gates**:
  - `project::findings_session::SESSIONS` and `project::session_profile::PROFILES`.
  - `parse::fetch_gate::GLOBAL_GATE` — process-wide single-flight grammar fetch mutex, 10-minute `GRAMMAR_FETCH_COOLDOWN`, and per-language `locally_failed` set.
  - `evidence::parse_worker` (`thread_local!` `ParseWorker`) — persistent per-caller-thread parse worker for non-Rust `tree-sitter-language-pack` `process()` calls.
  - `tools::grammar::COMPILED_QUERY_CACHE` — bounded cache of compiled `Arc<tree_sitter::Query>` instances shared across `structure` and `mechanic`.

### Dev-Only Workspace Crates & Harnesses (Not Shipped)
- `crates/gates` (`slugaudit-gates`, `publish = false`): repository quality-gate binaries (`check_source_limits`, `check_no_duplicates`, `check_producer_coverage`, `check_mcp_surface`) and policy tests (`crates/gates/tests/`).
- `crates/test_check` (`slugaudit-test-check`, `publish = false`): panic-free `TestResult` assertion helpers (`Check`, `CheckErr`, `ensure`, `ensure_eq`, `ensure_ne`).
- `fuzz/` (`slugaudit-mcp-rust-fuzz`, standalone `[workspace]` with `fuzz/Cargo.lock` and `fuzz/deny.toml`): `libfuzzer-sys` targets `request_arguments`, `structure_query`, and `query_sql`.

---

## 2. The Parts (`src/` Modules & Responsibilities)

| Module / Path | Responsibility |
| :--- | :--- |
| `src/main.rs`, `src/cli.rs`, `src/cli/usage.rs` | Binary entrypoint, `args_os` parsing into `cli::Command`, stderr tracing initialization (`SLUGAUDIT_LOG_FORMAT=text\|json`), `project::validate_startup_config`, signal handling, and pipe-safe stdout/stderr writers (`cli::out`, `cli::err`). |
| `src/install/mod.rs`, `src/install/path.rs`, `src/install/place.rs` | Resolution of `slugthug_home`, `slugaudit_dir`, `config_home`, and `running_binary`; `create_new(true)` staged binary copy, `slugaudit.prev` retention, atomic rename, and `$SHELL` `PATH` append. |
| `src/connect/mod.rs`, `src/connect/*.rs` | `ConnectError` and client MCP registration/removal: 23-agent catalog (`agents.rs`, `registry_types.rs` with `AgentDef`, `agent_probe.rs`), CLI dialect execution (`cli.rs`, `exec.rs` for connect & disconnect via `probe::cli_installed`, `run.rs` over `util::run_bounded_command`), quote-aware `.jsonc` handling (`jsonc.rs`), atomic JSON config updates (`config_file.rs`, `config_atomic.rs` over `util::write_atomic_chunks_with`), interactive connect/disconnect pickers (`interactive.rs`), and manual config guidance (`guidance.rs`). |
| `src/update.rs`, `src/update_fetch.rs`, `src/update_proc.rs`, `src/update_verify.rs` | Self-update against GitHub releases: `curl -q` fetch (`update_fetch.rs`), bounded subprocess wrapper (`update_proc.rs` over `util::run_bounded_command`), SemVer version comparison + `installed_version` + `sha256sum` check (`update_verify.rs`), and staged atomic replacement. |
| `src/menu.rs` | Interactive 6-option terminal setup menu and ANSI-styled rendering. |
| `src/server.rs`, `src/server/handlers.rs`, `src/server_runner.rs`, `src/server_progress.rs`, `src/resources.rs` | `SlugAuditServer` (`rmcp::ServerHandler`), `INSTRUCTIONS`, `#[tool_router]` registration of the 10 MCP tools, 8-permit `spawn_blocking` execution + per-call tracing, monotonic `ProgressSession`, and compile-time embedded `slugaudit://` resources. |
| `src/tools/` | Shared tool infrastructure (`context.rs`, `paginate.rs`, `paginate_sink.rs`, `paginate_store.rs`, `page.rs`, `page_size.rs`, `grammar.rs`, `tool_limits.rs`, `input_caps.rs`, `tool_counters.rs`, `language_notices.rs`, `sql_predicates.rs`, `project_control.rs`) and per-tool subdirectories: `query/`, `leads/`, `structure/`, `mechanic/`, `finding/`, `report/`, and `health/`. |
| `src/sync/` | `SourceSyncManager` (`manager.rs`, `manager_orchestration.rs`, `manager_meta.rs`), `ResultStoreSlot` (`result_stores.rs`), single-pass parallel walk + binary sniff + BLAKE3 hashing (`discovery.rs`, `file_pool.rs`, `hash.rs`, `hash_batch.rs`), manifest diffing (`manifest.rs`, `publish_diff.rs`), uncontended Pass 2 sampling/parsing/staging outside `BEGIN IMMEDIATE` (`sample.rs`, `sample_batch.rs`, `analyze.rs`, `classify.rs`), and atomic CAS revision publication + incremental dependency-edge resolution (`publish.rs`, `publish_attempt.rs`, `revision.rs`, `revision_edges.rs`). |
| `src/store/` | SQLite connection setup (`connection.rs`, `connection_config.rs` — WAL, `foreign_keys = ON`, 5s `busy_timeout`), `PRAGMA user_version = 13` schema (`migrations.rs`, `schema.sql` with `idx_evidence_item_name`, `idx_evidence_call_callee`, and trailing `content TEXT` on `files`), custom SQL functions (`sql_functions.rs`), session findings DB (`findings_db.rs`, `findings_schema.sql`), and private paged-result store (`result_store/`). |
| `src/parse/` | Language detection, prose/ignore-file classification, `PACK_ALIASES` (`language.rs`; `TESTED_LANGUAGES = &["rust"]`), deadline-bounded `tree-sitter` parser wrapper (`budget.rs`), `PACK_VERSION`, and single-flight grammar download cooldown gate (`fetch_gate.rs`, `local_load_failures.rs`). |
| `src/evidence/` | Typed `EvidencePayload` extraction (`normalize.rs`, `sql.rs`): **single-parse direct `tree-sitter-rust` CST extractor** (`rust_items.rs`, `rust_doc.rs`, `rust_bindings.rs`, `rust_mod_paths.rs`, `rust_path_attr.rs`, `calls.rs`, `calls/*`) for Rust, and `tree_sitter_language_pack::process` (`parse_worker.rs`) + shared iterative CST walkers (`bindings.rs`, `generic_imports.rs`, `calls.rs`) bounded at `MAX_WALK_DEPTH = 512` for other languages. |
| `src/graph/` | Import-source and Rust `mod` declaration resolution (`mod.rs`, `resolve_rust.rs`, `rust_crate_anchor.rs`, `cargo_workspace.rs`, `rust_mod_decl.rs`, `rust_paths.rs`, `rust_use_tree.rs`, `rust_mod_parents.rs`, `rust_super.rs`, `resolver/`) producing `ResolvedEdge`s across Cargo workspace crates, `mod foo;`, `#[path]` `super::` chains, and grouped `use` trees. |
| `src/project/` | Canonical root & `.planning/slugaudit/` activation (`activation.rs`, `gitignore.rs`), `GlobalConfig` & startup validation (`global_config.rs`, `limits.rs`), `findings_session.rs`, per-project `session_mechanics.rs`, and `session_profile.rs`. |
| `src/ignore_rules.rs` | `ignore::WalkBuilder` setup and hardcoded exclusions (`.git`, `.planning/slugaudit`, scratch filenames). |
| `src/model/` | Typed domain contracts and bounds: `EvidenceItem`, `EvidenceKind`, and typed `EvidencePayload` structs (`evidence.rs`), 1-based `Span` / fallible `LineIndex::new` (`span.rs`), `ParserRun` enums (`parser.rs`), `AuditProfile` (`profile.rs`, `max_sync_wall_clock` `30s`/`90s`), `ResourceLimits` / `LimitOverrides` (`limits.rs`, `limit_overrides.rs`), `EnvOverrides`, and host RAM detection (`system_ram.rs`). |
| Shared support | `src/error_codes.rs`, `src/clock.rs`, `src/progress.rs`, `src/probe.rs` (`which` + `cli_installed` Amazon Q verification), `src/prompt.rs`, and `src/util.rs` + `src/util/{atomic_write.rs, capped_read.rs, proc.rs, style.rs, test_capture.rs, time.rs}`. |

---

## 3. How They Talk to Each Other (Request & Sync Data Flow)

### MCP Tool Call Flow
1. **Transport & Router**: JSON-RPC requests arrive on stdin via `rmcp` -> `SlugAuditServer` (`src/server.rs`). `resources/list` and `resources/read` serve compile-time embedded `CATALOG` entries (`src/resources.rs`) with no disk or DB access. `tools/call` routes through `#[tool_router]` in `src/server/handlers.rs`.
2. **`health` (`SlugAuditServer::dispatch_health`)**:
   - Runs on `tokio::task::spawn_blocking` without taking a `blocking_ops` semaphore permit.
   - Calls `tools::health`: never syncs and never mutates; opens `project.db` read-only in a `DEFERRED` transaction to report `phase`, `remedy`, `database`, and typed `HealthDetail` (`database`, `database_read_failed`, `database_read_error`, `server`, `system_memory`, `system_memory_error`, `result_store`) when `verbose: true`.
3. **All Other 9 Tools (`SlugAuditServer::dispatch` -> `server_runner::run_blocking`)**:
   - Acquires 1 of 8 `blocking_ops` permits, emits monotonic `ProgressSession` notifications (`0.0 -> ... -> 1.0`), bumps `ToolCounters`, and logs once on completion.
   - **Paged continuations (`cursor` supplied)** on `query`, `leads`, `structure`, `finding_read`, and `mechanic` (`list`): read the next byte-bounded page from `ResultStore` at the original `revision_id` without syncing, and populate `page.current_disk_revision_id` when readable from `project.db`.
   - **Fresh calls**: run `tools::context::ensure_synced` -> `SourceSyncManager::ensure_current`, then read inside `with_verified_read` (`BEGIN DEFERRED` verifying `current_revision_id == synced.revision_id`). Single-page results return directly from memory without creating a `ResultStore` SQLite file; multi-page results spill into `ResultStore` and return `page.cursor`.

### Every-Call Sync Pipeline (`SourceSyncManager::ensure_current`)
1. **Root Resolution, Singleflight Lock & Session Init**: resolves `<root>/.planning/slugaudit/`, acquires the per-project singleflight mutex, and initializes `findings-<id>.lock` (sweeping dead sessions' findings files).
2. **Database Open, Metadata Check & Auto-Rebuild**: opens `project.db` (`PRAGMA user_version == 13`) and verifies singleton `project` metadata (`root_path`, `contract_version = 1`, `schema_version = 1`); missing, corrupt, or mismatched databases are discarded and rebuilt, reporting `DatabaseDiscard`.
3. **Pass 1 — Walk, Parallel Sniff+BLAKE3 Hash & Manifest Diff** (no stored-content re-read):
   - Walks `root` pruning `.git`, `.planning/slugaudit`, and ignored/scratch paths.
   - Worker pool (`min(files, available_parallelism, 8)`) opens each regular file once, sniffs the first 8,000 bytes for NUL (`indexed` vs `binary`), and streams BLAKE3 (`blake3-v1`) in 64 KiB chunks; oversize/unreadable files record `skipped` rows with `stat-identity`.
   - Compares disk `(path, content_hash)` against `SELECT path, content_hash FROM files` in SQLite (never re-reading or re-hashing stored `files.content`). If nothing changed and `parser_pack_version == PACK_VERSION`, returns the current `revision_id` immediately without writing.
4. **Pass 2 — Uncontended Chunked Parse/Staging & Atomic CAS Commit**:
   - Samples, parses (single-parse `extract_rust_cst` + `calls::extract` for Rust; persistent `ParseWorker` + CST walkers for other languages), TOCTOU-revalidates, and stages changed files in bounded 256-file chunks **outside** `BEGIN IMMEDIATE` on `project.db` so readers are not blocked during parsing.
   - Acquires `BEGIN IMMEDIATE` on `project.db`, verifies the CAS baseline revision (`MAX_CAS_RETRIES = 4`), applies staged file/evidence updates and deletions, resolves `dependency_edges` (`graph::build_workspace_crates`, `resolve_imports_full`, `resolve_rust_mod_declaration`, plus affected `#[path]` / `Cargo.toml` dependents), flips `revisions.is_current`, and commits.

### SQLite Schema & `query` Execution
- **`project.db` (`src/store/schema.sql`, `PRAGMA user_version = 13`)**:
  - `project`, `revisions` (`idx_revisions_single_current`), `unrepresentable_paths`, `files` (`idx_files_path_hash`, `idx_files_language_path`, trailing `content TEXT`), `evidence` (`idx_evidence_kind`, `idx_evidence_file`, `idx_evidence_item_name`, `idx_evidence_call_callee`, `idx_evidence_rust_mod_path`), and `dependency_edges` (`idx_edges_from`, `idx_edges_to`).
- **Scalar helpers & bounds**: registers `line_count(text)` and `line_range(text, start, end)`; `query` opens a read-only connection, refuses `PRAGMA` before preparation, wraps one `SELECT`/`WITH` statement as `SELECT * FROM (\n{body}\n) LIMIT -1 OFFSET 0`, and enforces `SQLITE_LIMIT_LENGTH`, `SQLITE_LIMIT_SQL_LENGTH`, `max_query_vm_steps`, and `max_query_wall_clock`.
