# Changelog

All notable changes to SlugAudit are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The newest entry MUST match the version in `Cargo.toml`.

## [Unreleased]

<!-- Entries for the next release go here. -->

### Fixed

- The interactive connect menu no longer prints "not valid JSON" warnings for
  clients whose config is TOML or YAML (codex, grok, hermes); those configs are
  now detected by a text scan with no format warning.
- Store-open and sync-sweep WARN logs now name the real path and the full OS
  error instead of a hashed path id and an error kind, so a failed open can be
  diagnosed from the log alone.
- `project_control` with `action: "off"` now reports `findings_discarded`, the
  number of findings in the project database it deleted (0 when there was none).
  If the count cannot be read, the call fails and the project stays enabled.
- `--help`/`-h` anywhere in a subcommand's arguments now prints that
  subcommand's usage and exits 0 without running it. Previously `update --help`
  performed a binary update, `serve --help` started the server, and
  `connect --help` treated the flag as an agent name.

## [1.1.0] - 2026-10-28

Covers the changes since `v1.0.10`. `Cargo.toml` records
`version = "1.1.0"`. The MCP wire surface changed more than in any earlier
release: read the *Breaking* entries under Changed before upgrading a client or
a saved prompt. The on-disk index schema moved from version 3 to 11; an
older database is discarded and rebuilt from source on first open, with no
migration step.

### Added

- `leads` tool: unranked mechanical facts about a project (call sites by callee
  such as `unwrap`, `expect`, `panic`, `todo`, `unimplemented` and `assert`;
  diagnostics; `TODO`/`FIXME`/`HACK` markers; credential-shaped files). It returns
  counts by default and file/line rows for one mechanic on request, can be
  narrowed with a path `pattern`, and takes `mode: "compact"` to return the whole
  catalogue as `mechanic`/`file`/`line` triples in one response. Row pages
  have no fixed row cap; credential-shaped file detection covers root-level
  `.netrc`, `.npmrc`, `.git-credentials`, `id_rsa`, `id_ed25519`, `id_ecdsa`,
  `*.p8`, `*.jks`/`*.keystore`, `*.tfstate` and `service-account*.json`, and does
  not count public keys such as `id_rsa.pub`; clipped `detail` text is flagged
  with `detail_clipped`.
- `mechanic` tool: `add`, `remove` and `list` the auditor's own mechanics, each a
  Tree-sitter query with one capture over one language, counted by `leads` and
  listed in `report` beside the built-ins (`source` is `builtin`, `project_config`
  or `session`). Session entries end with the server; `persist: true` writes the
  entry to the versioned `mechanics` section of `.planning/slugaudit/config.json`.
  A built-in can never be removed, renamed or overridden, and an invalid
  persisted entry is listed with its error rather than dropped. Scans of custom
  mechanics are cached per revision.
- Call evidence for Python, JavaScript, TypeScript, TSX, Go, Java and C through
  embedded call queries (`examples/calls/*.scm`), so `leads` counts them as it
  does Rust; each count's `covers` lists the languages it actually read. Rust
  macro-argument calls, including nested macros, are recovered.
- MCP resources: the schema, the `evidence.payload` key reference, and a
  cookbook of query recipes under `slugaudit://` URIs, served by `resources/list`
  and `resources/read`. Output schemas, titles and annotations now exist for every
  tool, and `check_mcp_surface` pins the whole surface against
  `.planning/archive/mcp_surface.json`.
- `query` pages at `default_page_bytes` (64 KiB; `SLUGAUDIT_DEFAULT_PAGE_BYTES` or
  `config.json`) with a per-request `page_bytes` override and an exact total.
  Oversized cells are clipped with a ready `substr` continuation reported in
  `clipped_cells`, and the SQL functions `line_count(text)` and
  `line_range(text, start, end)` are available on every connection.
- Stored paged results: the first call of `leads`, `query`, `structure`,
  `finding_read` and `mechanic list` writes its rows and totals to a result store
  under `.planning/slugaudit/results/`, and every page describes that one
  snapshot however the files change meanwhile. New limits
  `SLUGAUDIT_MAX_RESULT_STORE_BYTES` (4 times `max_query_response_bytes` by
  default) and `SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS` (900), also `config.json` keys.
  New error codes `unknown_cursor`, `result_expired`, `result_store_full`,
  `result_too_large`, `result_store_unavailable` and `paging_parameter_removed`;
  `health` `verbose` reports the store under `detail.result_store`. See
  `docs/TROUBLESHOOTING.md`.
- `structure` pages matches by bytes, reports `files_not_scanned` with a
  `next_start_after_file` continuation when the multi-file memory window cuts a
  scan, and bounds its query text (`SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES`, 16 KiB).
- `files_not_parsed` on `leads` and `report`; `report.completeness` with counts by
  file kind, skip reasons and extraction completeness. Every discovered file now
  has a row in `files`, including skipped (oversize or unreadable) ones, with a
  `skip_reason` code.
- Parse limits: `SLUGAUDIT_MAX_PARSE_BYTES` (default 2 MiB; larger files are
  stored but not parsed) and `SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS` (per-file parse
  allowance, default 5000 ms; a timed-out file is counted and the sync
  continues). `SLUGAUDIT_MAX_WATCH_DEPTH` and `SLUGAUDIT_MAX_WATCH_ENTRIES` bound
  the watcher's scope. Every overridable limit refuses 0: from an environment
  variable it stops startup, from `config.json` it is ignored with one warning.
- Machine-readable error payloads (`data.code` and a static `remedy`) on the
  failures a caller can act on, including `stale_revision`, `sql_error` (from
  `query`) and `unknown_language` (from `structure`).
- `project_control {"action":"on"}` appends `.planning/slugaudit/` to the
  project's `.gitignore` unless an equivalent rule is present; `off` leaves the
  file alone.
- `finding` collapses a restated conclusion within a session into one row
  (same path, source hash, lines, category and title) instead of inserting a
  duplicate.
- Every tool call carries a process-local `call_id` and emits one completion line
  (tool, elapsed time, outcome), so a session's call sequence can be recovered
  from the log. Arguments and results are never logged.
- `slugaudit install` and `slugaudit update` keep the binary they replace as
  `slugaudit.prev` beside the new one, and refuse the replacement if that copy
  cannot be made; `install.sh` does the same. `slugaudit update` fetches over
  HTTPS only and refuses unsupported platforms.
- Release artifacts: an SBOM, third-party notices and a signed build-provenance
  attestation, with a rebuild-and-compare check; see `docs/RELEASING.md`.
- `[profile.release]` with `strip = true`, `lto = "thin"` and `codegen-units = 1`.
- `finding_sessions` tool: `list` shows the sessions that have findings in the
  database and `adopt` brings an earlier session's findings back into the
  current session, but only when the finding's `source_hash` still matches the
  file.
- `database_discard` field on the response of the call that discarded a corrupt
  or incompatible database: `findings_discarded` (counted before removal; `null`
  when the database was unreadable, never a fabricated 0) and the `cause`. The
  same call logs one WARN.
- `slugaudit` exits with status 2 on usage errors, accepts `-v`, exits 0 on
  SIGTERM and SIGINT, and exits 0 with one log line when stdin closes before
  `initialize`.
- `NO_COLOR` and `TERM=dumb` are honoured by the CLI output.

### Changed

- **Breaking:** paging moves to an opaque `cursor`. `offset` and
  `expected_revision` are removed from `leads`, `query`, `structure`,
  `finding_read` and `mechanic`, and a request that still sends either is refused
  with `paging_parameter_removed`. Every response carries `page.cursor` (absent on
  the last page) and `page.completeness` (`complete`, or `partial` with
  `limited_by`, `resume_after_file`, `files_scanned` and `files_total`); to
  continue, repeat the call with only that `cursor`. A continuation does not sync,
  so it returns the rows, totals and `revision_id` of the first page. `next_offset`
  is gone from `page`. `structure` no longer continues a window-limited scan with a
  second cursor: a partial result is resumed by a new request carrying the
  `start_after_file` request parameter (the result's `resume_after_file`, also
  reported as `next_start_after_file`), which syncs. `query` is all-or-nothing: the
  whole statement is stored or the call fails, so `total` is always exact and
  `total_unavailable` and its separate count budget are removed.
- **Breaking:** one paging contract. `query`, `structure`, `finding_read`, `leads`
  and the `mechanic` list/count responses carry a shared `page` object with
  `returned`, `total`, `complete`, `cursor`, `page_bytes`, `bytes_used`,
  `limited_by` and `revision_id`. Against 1.0.10 this removes the top-level
  `truncated`, `next_offset` and `revision_id` from `query`, `truncated` and
  `revision_id` from `structure`, and `revision_id` from `finding_read`; read
  them from `page` instead. `structure` and `finding_read` now page, and
  `finding_read` reports `complete`.
  A request above the page ceiling is clamped, and `page_bytes` already
  reflects the clamp. `limited_by` is one of `none`, `limit`, `page_bytes`,
  `ceiling`, `match_cap`, `scan_window` or `time_budget`. Tool-specific extras (`clipped_cells`,
  `non_finite_cells`, `oversized_row`, `files_total`, `files_covered`,
  `files_not_parsed`, `files_not_scanned`, `bytes_not_scanned`,
  `next_start_after_file`) sit beside `page`. `query`'s `non_finite_cells`
  lists each REAL cell that is `null` only because JSON cannot carry an
  infinity or NaN (`row`, `column`, `value`); a `null` not listed is SQL NULL.
  `structure.bytes_not_scanned` is the content size of the files not scanned.
- **Breaking:** line numbers are 1-based and inclusive on every surface
  (`leads`, `query` evidence rows, `structure`, `finding`, `line_range`). The
  stored `evidence.start_line`/`end_line` change meaning, so existing indexes
  rebuild.
- **Breaking:** `health` is grouped by source and answers only `phase`,
  `remedy`, `watcher.health`, `database.revision_id` and `database.file_count`
  unless called with `verbose: true` (new input). Against 1.0.10 the flat
  `watcher_health`, `revision_id` and `file_count` are now `watcher.health`,
  `database.revision_id` and `database.file_count`. These 1.0.10 fields moved under
  `detail`, present only with `verbose: true`: `pending_dirty`,
  `pending_deleted`, `watcher_sequence` (now `sequence`) and
  `last_verified_sequence` under `detail.watcher`; `parser_pack_version` under
  `detail.database`; the tool-call counters, `last_sync_unix_seconds`,
  `last_sync_duration_ms` and `consecutive_full_publishes` under
  `detail.server`; and `system_memory` under `detail`. New, also behind
  `verbose`: `skipped_file_count` under `detail.database`, and
  `database_read_failed` and `mechanic_cache` under `detail`. The database
  fields are all present or `database` is null.
- **Breaking:** `structure` over a whole project for a language no indexed file
  has now fails with `unknown_language` (the message lists the indexed
  languages) instead of returning an empty result or the grammar pack's
  "Invalid node type". `structure` refuses prose files.
- `query` pages are cut by `page_bytes` and the response-size ceiling, and a
  caller who wants fewer rows writes `LIMIT`. A
  statement list is rejected by SQLite's own statement count rather than a
  hand-written scanner, so `SELECT 1; -- done` is accepted. Duplicate result
  column names are rejected.
- Prose (Markdown, reStructuredText, plain text, ignore-pattern dotfiles) is
  stored with `language = 'prose'` and never run through a grammar, so it
  produces no parser failures; it is counted in
  `completeness.prose_files_not_parsed`.
- A file whose grammar download failed is recorded as `LoadFailed` and retried
  after a 10-minute cool-down (or on the next server start) instead of staying
  unparsed. Grammars are downloaded lazily on first use; see the README.
- `extraction_completeness` is `ContentOnly` for indexed-but-unparsed files
  instead of `Unavailable`, and `report` parser-failure rows carry
  `extraction_completeness` and `parser_availability`. The reasons
  `oversize_for_parse` (a file above `SLUGAUDIT_MAX_PARSE_BYTES`) and
  `parse_timeout` (a parse cut at its deadline) are recorded on those files.
- The first `report` of a server carries a `mechanics.note` explaining how
  mechanics work; later reports omit it.
- The server instructions were rewritten and cut from 12,561 to 5,474
  characters: paging is described once, with a `MECHANICS` paragraph and a
  line-numbers statement. The `query` description lists all eight evidence kinds.
- `slugaudit connect` rebuilt its client registry from probed binaries: config
  paths resolve against the user's home, CLI-registered clients are probed at
  call time, and every failure ends with a paste-ready remedy. The registry is
  now 23 clients; a non-object
  config section, an unrepresentable binary path and a failed `mcp remove` are
  refused with a typed, truthful message instead of reporting success.
- `health` reports an indexed project without a registered watcher as its own
  state, and answers normally when no project is active. Every tool reports the
  disabled state with one actionable instruction.
- Dependency updates: `rmcp` 3.5, `tree-sitter-language-pack` 1.18.0, `tokio`
  1.53.3, `criterion` 0.8, `thiserror` 2.0.21, `uuid` 1.28, `which` 8.0.6.
- `unwrap` and `expect` are denied outside tests (`clippy::unwrap_used`,
  `clippy::expect_used`).
- **Breaking:** a project is active if and only if
  `.planning/slugaudit/project.db` exists as a regular file; the directory alone
  no longer activates it. `project_control` `off` now deletes `project.db`, its
  `-wal`, `-shm` and `-journal` sidecars, the `results/` directory and the lock
  file, keeps `config.json` and any other file, and removes
  `.planning/slugaudit/` only when it is then empty. The response lists what was
  removed (`removed`) and that `config.json` was kept (`config_kept`). After
  `off`, every tool call returns `project_not_enabled` instead of rebuilding the
  database; `on` after `off` restores the persisted mechanics from
  `config.json`. A call already in flight when `off` runs also returns
  `project_not_enabled` and does not recreate the database; the one remaining
  window is documented in `.planning/ARCHITECTURE.md`.
- Findings from earlier sessions are marked `stale` on the first sync of each
  process instead of being deleted; `finding_read` and `report` count only the
  current session.
- The default log filter is `warn,slugaudit_mcp_rust=info,rmcp=error`, so
  dependency chatter no longer reaches stderr. Failures log the real path and
  the source error once.
- **Breaking (startup):** an invalid `SLUGAUDIT_*` limit variable, a limit
  variable set to `0`, an invalid or non-UTF-8 `RUST_LOG`, and a non-UTF-8
  `SLUGAUDIT_LOG_FORMAT` now stop startup with exit 1 naming the variable, where
  they used to warn and continue with defaults. A malformed project
  `config.json` fails the call that reads it. Without a detectable system memory
  size an explicit profile is required, and `health` reports `system_memory` as
  `null`. A `0` in `config.json` is still ignored with one warning.
- `slugaudit install` and `slugaudit menu` reject extra arguments with a usage
  error.
- `config.json` rewrites keep key order, the file mode and a trailing newline.
- `install.sh` and `slugaudit update` hardening: the shell profile edit refuses
  an unknown `$SHELL`, the previous binary is kept atomically as `.prev`,
  `install.sh` quotes and traps cleanly and its cargo fallback installs to the
  chosen target path, a malformed release asset entry is an error, and a partial
  download is removed.
- Progress notifications are sent on the server's runtime handle.

### Removed

- The 500-row cap on `query` pages.
- The `pearai`, `trae` and `1mcp` clients from the `slugaudit connect`
  registry.
- From `query`, the top-level `truncated`, `next_offset` and `revision_id`; from
  `structure`, `truncated` and `revision_id`; from `finding_read`,
  `revision_id`. They are on `page`.
- The `-1` sentinel values in `health`.

### Fixed

- Sync, watcher and publish paths fail closed on stat, read, WAL and discovery
  errors instead of silently dropping files or events; an incomplete sweep is
  not published as complete, and prior findings are purged only at session
  start. The watcher's error callback no longer deadlocks, and watches are
  registered one scoped directory at a time so `max_watch_entries` bounds
  kernel watches.
- Projects with more than 32,766 files index correctly (the reconcile query
  binds one JSON path array instead of one parameter per file).
- Two servers opening a fresh project database at the same time no longer fail
  one of them with `database is locked`.
- Import resolution: imports of discovered-but-skipped files are no longer
  reported as unresolved dead code; relative and root-escaping imports
  resolve or stay unresolved correctly.
- Rust turbofish calls (`f::<T>()`) are named correctly, and calls inside macro
  token trees are no longer dropped.
- A deeply nested file truncates instead of aborting the process; tree walks
  run on an explicit heap stack and are bounded.
- Network filesystems, including on macOS, are detected from the mount table so
  a fresh database can open; the index directory is created owner-only.
- A non-UTF-8 command-line argument is rejected instead of panicking; prompt
  input, foreign file reads, the `.gitignore` read (1 MiB) and tool inputs are
  bounded.
- Messages that asserted outcomes the code had not established now tell the
  truth: `connect`, `update`, `health`, `report`, `project_control` and
  `structure` source errors, and a failed install removes its staged binary.
- Logs no longer print absolute paths or user content; swallowed errors in
  sync, watch, install, connect, update and logging are logged or propagated.
- Dev gates (`check_*`) fail closed when an input is unreadable, a root is empty
  or a report is stale, instead of printing PASS.
- A call carrying a `progressToken` (seen with `query` and `project_control`) no
  longer fails with `-32603` "a scoped thread panicked" when its sync runs on a
  worker thread.
- The log-and-continue fallback in `process_limits()` is gone: a limit
  environment that was not validated at startup is now an error, not a silent
  default.

### Security

- `query` refuses `PRAGMA` at prepare time. A statement such as
  `PRAGMA hard_heap_limit=1` could change process-wide SQLite state from a
  read-only tool call; it is now denied by the authorizer (the
  `pragma_table_info` family of table-valued reads still works), and the refusal
  message no longer says "only comments". SQLite's length limit is set on the
  read-only connection, so the length of a value is bounded (by the larger of
  `max_query_value_bytes` and 3 x `max_file_bytes`) and a
  `zeroblob`/`hex`/`replace` amplifier past it fails with an error.
- `project_control` and the `config.json`/`.gitignore` writers refuse to write
  through a symlink or a non-regular file, so a link planted in a project can no
  longer redirect a write outside it.
- Every `config.json` write goes through one locked, atomic read-modify-write
  that preserves unrelated keys, so concurrent writers cannot corrupt or lose
  each other's changes. A crash-orphaned `config.json.*.tmp` is removed on the
  next load.
- `rustls-webpki` and `webpki-roots` were updated for RUSTSEC-2026-0382.
- Tool inputs are bounded and operational logs are redacted; `slugaudit update`
  fetches over HTTPS only.
- The install and update paths refuse to continue on a malformed asset entry or
  an unknown shell, and a gitignore edit syncs its parent directory.

### Known issues

- The `zeroblob`/`hex`/`replace` amplifier can still reach about 1.5 GB
  (estimated) on the `audit` profile; a lower stored-content default is under
  review.

## [1.0.10] - 2026-09-06

Release commit `release: bump version to 1.0.10 for hardware-adaptive scaling
and profile release`, tagged `v1.0.10` on 2026-09-06. `Cargo.toml` records
`version = "1.0.10"`.

### Added

- Hardware-adaptive scaling and the `adaptive` / `audit` / `lean` resource
  profiles (`src/model/profile.rs`, `src/model/limits.rs`).

[Unreleased]: https://github.com/SlugThugLabs/slugaudit/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/SlugThugLabs/slugaudit/compare/v1.0.10...v1.1.0
[1.0.10]: https://github.com/SlugThugLabs/slugaudit/releases/tag/v1.0.10
