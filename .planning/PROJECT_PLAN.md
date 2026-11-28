# SlugAudit Project Plan

Last updated: 2026-11-26. `AGENTS.md` is the product definition; this file
records what is built on `main` and what decisions remain for the owner. The
owner approves targets before work begins and before anything is committed.

## Goal [approved]

Make the tool accurate and fast for Rust: every answer matches the code on
disk at the moment of the call, safely, on a normal laptop. Nothing else gets
built until that is true.

## What the Product Is [built]

- **Local stdio MCP server on `rmcp`:** SlugAudit runs over stdio on the `rmcp`
  SDK (`3.11.0`), which owns process startup, transport framing, session
  lifecycle, and shutdown. SlugAudit provides the SQLite index, the per-call
  sync pipeline, and 10 MCP tools: `query`, `report`, `structure`, `leads`,
  `mechanic`, `finding`, `finding_read`, `finding_clear`, `project_control`,
  and `health`.
- **Per-call disk synchronization:** On every tool call except `health` (and
  cursor continuations reading a stored page), before answering, SlugAudit
  walks the project, BLAKE3-hashes every regular file on disk, compares against
  `files.content_hash` in `<root>/.planning/slugaudit/project.db`, updates
  changed rows, adds new files, and deletes removed files. Stored
  `files.content` is never re-read or re-hashed out of SQLite. A missing,
  unreadable, root-mismatched, or schema-incompatible database is discarded and
  rebuilt from disk.
- **Host-harness trust boundary:** SlugAudit is not a security boundary. The
  host coding harness and OS own workspace permissions and already give the
  model direct access to repository files; `.planning/slugaudit/` (`project.db`,
  `findings-<id>.db`, `results/`) is disposable local scratch created with
  normal user permissions. SlugAudit bounds accidental runaway queries and
  parses (read-only SQLite connection on `query`, VM-step/time/byte caps).
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
  `~/.slugthug/slugaudit/config.json` (all 16 limit keys + `profile`; `0`
  rejected with `ConfigError::Zero`) and `SLUGAUDIT_*` environment variables.
  Tools never write them and model-facing text does not mention the global
  file. Limit precedence is profile baseline (`adaptive` by default, `lean`, or
  `audit`) -> `config.json` limit keys -> `SLUGAUDIT_*` overrides, with profile
  selection ordered `session_profile` > `SLUGAUDIT_PROFILE` > `config.json`.
- **Evidence-only surface and language scope:** Built-in `leads` count
  mechanical patterns only, with no ranking, severity, scoring, or checklists.
  Rust is the only tested language (statically linked into the binary); other
  languages download Tree-sitter grammars on first use, remain available, and
  emit untested-language `notices`.
- **CLI and distribution surface:** One shipped binary (`slugaudit` with
  `serve`, `install`, `connect`, `disconnect`, `update`, `menu`, `version`, and
  `help`), `install.sh`, and directory metadata (`lhm.plugin.json`,
  `glama.json`).

## Shipped Milestones [built]

### 2026-11-26 — Single-Parse Rust CST, Uncontended Pass 2 Sync, C5 Slimming, Host-Harness Reset, and Cookbook Dogfooding [built]

- **Single-parse Rust CST & typed `EvidencePayload`:** Replaced `process()` +
  secondary enrichment re-parses on Rust with a single-parse iterative
  `tree-sitter-rust` CST extractor (`src/evidence/rust_items.rs`,
  `src/evidence/normalize.rs`) and typed `EvidencePayload` structs
  (`src/model/evidence.rs`), covering `pub` / `pub(crate)` visibility, outer
  `///` docs without `//!` module-doc leakage, `Trait` + body-less `Method`
  signatures, `Union`, `static`, destructured `let` bindings, macro-interior
  calls, external `mod foo;` + `#[path]`, and Cargo workspace crate resolution.
- **Sync & SQLite slimming:** Removed per-call stored-content re-hashing,
  merged `hash_io.rs` into `src/sync/hash.rs`, staged Pass 2 256-file chunks
  outside `BEGIN IMMEDIATE` before atomic CAS publication, added
  `idx_evidence_item_name` (`PRAGMA user_version = 13` in `src/store/schema.sql`),
  and set `max_sync_wall_clock` defaults to `30s` (`lean`) / `90s` (`audit` and
  `adaptive`).
- **C5 slimming & module consolidation:** Removed the in-memory mechanic scan
  cache, invalid-mechanic tracking, coverage and performance gate binaries, and
  Criterion benchmark targets; scoped `SessionMechanics` per project and
  simplified `ResultStoreSlot`; shared `util::run_bounded_command` across
  `connect/run.rs` and `update_proc.rs` and `util::write_atomic_chunks_with` in
  `connect/config_atomic.rs`; consolidated `menu.rs`, `connect/mod.rs`
  (`ConnectError`), `connect/registry_types.rs` (`AgentDef`), `connect/exec.rs`
  (`probe::cli_installed` preventing `/usr/bin/q` collisions),
  `connect/interactive.rs`, quote-aware `connect/jsonc.rs` (refusing to erase
  comments in `.jsonc` configs), and `update_verify.rs`.
- **CI, fuzz, integration tests & docs:** Added `fuzz/deny.toml` (`--config
  fuzz/deny.toml` in CI), renamed `osv-scanner.toml` (`--config=osv-scanner.toml`),
  appended `tree-sitter-rust` MIT attribution to `THIRD-PARTY-NOTICES.txt` in
  `release.yml`, passed `TSLP_LINK_MODE=static TSLP_LANGUAGES=rust` on source
  builds in `install.sh`, added `< 1s` cold-start `initialize` check in
  `tests/stdio_protocol.rs`, added Linux `VmHWM <= 512 MiB` peak-RSS and
  warm/incremental bounds in `tests/large_repo_soak.rs`, added
  `tests/rust_cookbook_dogfood.rs` executing every `examples/queries/*.sql`
  recipe, and aligned all `docs/` with the live host-harness scratch model.

### 2026-11-25 — Rust Import Resolution, Metadata Error Propagation, and Fallible LineIndex [built]

- **Grouped `use` declarations and `#[path]` `super::` resolution:**
  Expanded grouped/nested Rust `use` trees (`src/graph/rust_use_tree.rs`) and
  resolved `super::` imports inside `#[path]` files (`src/graph/rust_mod_parents.rs`,
  `src/sync/revision_edges.rs`) with incremental dependent re-resolution.
- **Sync metadata `SELECT` error propagation and corrupt-DB rebuild:**
  Propagated SQLite corruption in `ensure_project_row` (`src/sync/manager_meta.rs`)
  into automatic `rebuild_and_finish` recovery.
- **Fallible `LineIndex::new`:** Returned `Result<Self, PositionOverflow>`
  directly from `LineIndex::new` (`src/model/span.rs`).

### 2026-11-03 to 2026-11-17 — Guard Removal, Gates Crate, Zero-Panic Policy, Singleflight Sync, and rmcp 3.11 [built]

- Delegated `.planning/` setup to `slugplan::ensure`; removed unreported-threat
  guards (`0600`/`0700` file modes, symlink refusals, `NOFOLLOW`); moved the four
  `check_*` gate binaries (`check_source_limits`, `check_no_duplicates`,
  `check_producer_coverage`, `check_mcp_surface`) into dev-only crate
  `crates/gates`; enforced zero-panic/zero-unwrap lints and `TestResult` checks;
  serialized per-project syncs via `sync_locks`; capped open `ResultStore`
  entries at `MAX_OPEN_RESULTS = 64`; upgraded to `rmcp 3.11.0`.

### 2026-10-31 to 2026-11-02 — Two-Pass Sync, Session State, Contracts, and Listings [built]

- Established `.planning/slugaudit/` as the opt-in project switch, moved
  findings/mechanics/profile to session-only storage, added read-only
  `~/.slugthug/slugaudit/config.json`, implemented two-pass sync and
  result-store cursor paging, and aligned `lhm.plugin.json` and `glama.json`.

## Open Owner Decisions [draft]

- **Version and release cut:** `Cargo.toml` and `lhm.plugin.json` specify
  `1.1.0` while the latest git tag is `v1.0.10`. Cut a new release tag only
  when the owner approves shipping, updating external directory listings
  (Glama, LHM, and peer directories) on the same day so published metadata
  matches the tagged release.

## Explicit Non-Goals (Not Doing) [approved]

- No file watcher.
- No re-reading or re-hashing stored `files.content` out of SQLite (`verify_stored`).
- No security guards against unreported threats or against the model/user
  tampering with local scratch files (no SQL authorizer, `0600` file-permission
  policing, workspace path jails, symlink/`NOFOLLOW` refusals, locks beyond
  dead-session cleanup, or network-filesystem refusals).
- No in-memory mechanic scan cache (`mechanic_scan_cache`) and no `check_coverage`
  or `check_performance` gate binaries.
- No ranking, scoring, severity assignment, or checklists from built-in tools.
- No removing non-Rust language support or the `install.sh` / `install` /
  `connect` / `disconnect` / `update` / `menu` surface, and no new non-Rust
  tests or fixes unless a user reports a bug.
- No new features, layers, or hardening without the owner's OK.
