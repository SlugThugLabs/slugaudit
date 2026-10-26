# Deep dive of `main` at `c3fc7f8` (2026-10-25)

Evidence record for Phase 8 of `.planning/PLAN.md`. Everything here was
reproduced against a fresh `cargo build --release --locked` of `c3fc7f8`
(binary `slugaudit 1.0.10`) driven over stdio, or read from the code at the
cited `path:line`. Line numbers cite the tree at `c3fc7f8` and may drift.

Method: the owner's 11 CI gates were run (all pass: 1,256 tests, 0 failed,
1 ignored, 93.70% line coverage); the server was exercised against a copied
68k-line Rust workspace (10,369 files) and a 17k-line Python project; four
independent read-only reviews covered the query/store boundary, the
architecture, the model-facing surface, and per-language evidence coverage.

## Measured

| Measurement | Result |
|---|---|
| Full index, 68k-line Rust, 10,369 files | 8.2 s, 193 MB peak RSS, 49 MB `project.db` |
| Full index, 17k-line Python, 280 files | 0.41 s |
| Warm start to first `report` | 0.22–0.37 s (Rust), 28 ms (Python) |
| `leads` on the Rust project | ~0.3 s per call; `query` ~55 ms |
| Incremental: add a file with 3 `unwrap()` then `leads` | count +3 in 0.31 s; delete removes them |
| `assert_calls` vs `rg` on the Rust project | 7,038 vs 7,117; all 79 extras are comments/strings; no misses found |
| `comment_markers` vs `rg 'TODO\|FIXME\|HACK\|XXX'` | 1 vs 10; the 9 extras are colour codes, strings, regexes, prose |
| `initialize` instructions | 10,501 chars (~2.6k tokens) |
| `tools/list` wire content | 67,277 chars (~16.8k tokens); output schemas are ~50k of it (`report` 14.1k, `health` 10.6k, `leads` 9.1k) |
| 50 MB Rust file with 1.3M `unwrap()` | 185 s, 7.5 GB peak RSS on an 8 GB host; first attempt SIGKILLed; afterwards `leads` reported 0 unwraps with no warning |
| 50 MB `.txt` file | blocked the call for 58 s (parsed as `vimdoc`) |

## Confirmed defects

### D1 — refused PRAGMA takes effect at prepare time (P0)

- `src/tools/query.rs:279` (`single_statement_body`) calls `tx.prepare(sql)` on
  the raw SQL before wrapping it. SQLite applies `hard_heap_limit`,
  `soft_heap_limit` and `temp_store_directory` while preparing.
- Repro: `query {sql:"PRAGMA hard_heap_limit=1"}` returns
  `sql must contain a statement, not only comments` (misleading); every later
  call in the process, for every project, fails with
  `opening the project database for sync: failed to open database …` until
  restart. Reproduced twice independently.
- `src/tools/query_safety_tests.rs:112-155` only re-reads `user_version` and
  `journal_mode`, which apply at step time, so the class is untested.
- Related, same boundary: no `sqlite3_limit(SQLITE_LIMIT_LENGTH)` is set
  (grep `set_limit` in `src/store` finds nothing);
  `SELECT length(replace(replace(hex(zeroblob(250000000)),'0','00'),'00','000'))`
  peaked at 2.1 GB RSS inside the VM-step budget. `max_query_value_bytes` is
  only checked on result values.

### D2 — line numbers are 0-based on the wire

- `src/model/span.rs:4-7` documents zero-based lines; `src/evidence/normalize.rs:120`
  stores `span.start_line` raw; `leads`, `evidence.start_line/end_line`, and
  `structure` all return 0-based lines. `finding` (`src/server/handlers.rs:125`)
  and `line_range()` (`src/server.rs:94`) are 1-based.
- Repro: a 5-line `main.rs` with `x.unwrap()` on line 3 and `// TODO` on line 4
  gives `leads` rows `line: 2` and `line: 3`; `structure` gives
  `start_line: 0` for `fn main` on line 1. Nothing documents the convention.
- On the 68k-line project 4,930 apparent mismatches against `rg` became 0
  after adding 1 to every `leads` line.

### D3 — writes follow symlinks out of the project (P1)

- `src/tools/project_control.rs:241` `std::fs::write(&conf_path, …)` follows a
  symlinked `.planning/slugaudit/config.json`; the symlink check at
  `src/project/activation.rs:107-113` covers directories only. Repro:
  `config.json -> victim.txt`, then `on` with `profile:"lean"` replaced
  `victim.txt` with `{"profile":"lean"}`.
- `src/project/gitignore.rs:132,173` canonicalises `.gitignore` and atomically
  replaces the target with no containment check. Repro: `.gitignore -> victim_rc`
  appended `.planning/slugaudit/` to the target and gave it a new inode.
  `gitignore_tests.rs:203` pins the behaviour for an in-project target only.

### D4 — parsing is unbounded by size and by time

- The only per-file cap is `max_file_bytes` (`src/model/limits.rs:60`), sized
  RAM/10 and clamped to ≥256 MiB on the default `adaptive` profile
  (`src/model/profile.rs`), and it governs storage, not parsing.
- `parser.parse(content, None)` at `src/tools/structure_exec.rs:92` and the
  sampler's parse have no progress callback; the deadline is checked between
  files only (`structure_exec.rs:89`). `Query::new` on untrusted text
  (`src/tools/structure.rs:204`) has no deadline. The structure window check
  (`src/tools/structure_source.rs:140-141`) runs after the push, so one file
  up to `max_file_bytes` is loaded whole.
- Consequence measured above: 7.5 GB RSS, a SIGKILL, and a silent 0.

### D5 — prose parsed as code produces every diagnostic

- All 172 (Rust project) and 115 (Python project) `Diagnostic` rows came from
  Markdown and `.txt` files; none from source. The pack maps `.txt` to
  `vimdoc` and `.env` to `dotenv` (`src/parse/language.rs:5-7` defers to
  `detect_language_from_path` with no override).

### D6 — grammar download is undocumented and mis-classified

- `tree-sitter-language-pack` is built with default features `download` +
  `dynamic-loading`; `TSLP_LANGUAGES` is set nowhere; `nm` on the release
  binary and on the shipped 1.0.5 binary finds zero `tree_sitter_*` grammar
  symbols; both contain
  `https://github.com/xberg-io/tree-sitter-language-pack/releases/download`.
  Grammars arrive as a ~22 MB `.tar.zst` of `.so` files (sha256 pinned in a
  manifest fetched from the same release) into
  `~/.cache/tree-sitter-language-pack/v1.18.0` and load via `libloading`.
- `src/parse/language.rs:13` calls `has_language`, which is true for
  not-yet-downloaded grammars; the doc comment describes `has_parser`.
- `is_load_failure` (`src/sync/analyze.rs:49-58`) does not include the pack's
  download error, so an offline failure lands in the parse-failed bucket and,
  because the content hash does not change, is never retried.
- `PLAN.md` records runtime download as accepted (2026-10-19); no user-facing
  document mentions it, and the same list says "no network listener".

### D7 — call evidence is Rust-only

- `src/evidence/normalize.rs:80` gates the call walker on `language == "rust"`;
  `src/evidence/calls.rs:158-184` matches Rust node kinds only. All five
  callee mechanics return 0 with `covers: rust_only` on any other language.
- Known Rust gaps: turbofish callees (`generic_function`) fall through and
  the whole call text becomes `callee_name`; no name resolution (a user fn
  named `unwrap` counts); `#[cfg(test)]` is not distinguished.

### D8 — model-facing surface

- Paging dialects differ per tool: `complete` (leads) vs `truncated`
  (query/structure) vs neither (finding_read); byte counters are
  `row_budget_bytes`/`rows_bytes`, `rows_bytes`/`page_bytes`,
  `matches_bytes`/`page_bytes`, `findings_bytes`/`page_bytes`;
  `limited_by`, `oversized_row`, `files_covered` exist only on `leads`;
  `structure` has a second cursor (`next_start_after_file`).
- INSTRUCTIONS repeats the paging text three times and the judgment
  disclaimer at length; the `query` description lists four evidence kinds
  where eight exist; `examples/queries/README.md` shows `LIKE` on payload
  text, which INSTRUCTIONS forbids; `examples/queries/blast_radius.sql`
  holds three statements and fails as one `query` call.
- The first `report` immediately after `project_control on` returned
  `stale_revision … retry the call` (rev-2 → rev-3) on a 2-file project.
- SQL errors carry no `data.code`/`remedy`; an unknown `structure` language
  reports "Invalid node type".
- `health` (12k chars of schema) is operator-facing but served to the model.

### D9 — architecture notes (recorded, not all scheduled)

- The stat sweep (`src/sync/sweep.rs:94-118`) is the freshness guarantee; the
  watcher only closes a same-second same-size edit with no event. `Unavailable`
  forces a full publish every call (`src/sync/manager_health.rs:44-66`), so
  losing the optimisation costs more than never having it. ~1,666 production
  lines in `src/watch/` plus the barrier/health machinery defend it.
- No per-project in-process sync lock; 8 concurrent first calls each run a
  full parse and lose the CAS (`src/server_runner.rs:27`, `src/sync/sample_batch.rs:86`).
- `reconcile/queries.rs:95-106` (`touch_unchanged`) runs `UPDATE files` outside
  `publish_revision`, violating the single-writer rule in `ARCHITECTURE.md`.
- Six `sync` files return `rmcp::ErrorData` (e.g. `manager_orchestration.rs:17,105`).
- Fail-open spots: `ignore_rules/build.rs:159,188` (`.ok()` on a malformed
  ignore file, while `discovery.rs:60` fails closed on the same condition);
  `netfs.rs:165` `unwrap_or("")` on a malformed mountinfo line.
- `ARCHITECTURE.md:347-349` and `sweep.rs:5-6` say watcher events are
  "dropped"; `watch/manager/events.rs:64-67` defers and replays them.
- `src/util/capped_read.rs:66`: `metadata` then `File::open` lets a FIFO
  swapped in between block the sync (needs a local racing attacker).
- No `[profile.release]`; the 17.9 MB binary is unstripped symbols, not grammars.
- Release: latest tag `v1.0.10` (2026-09-06); 268 commits since; `CHANGELOG.md`
  stops at 1.0.10; `/root/slugaudit-release` holds a 1.0.5 binary whose
  `--help` names `~/.slugthug/bin/` while README names `~/.slugthug/slugaudit/`.

## Verified well

- Read-only connection opens `SQLITE_OPEN_READ_ONLY | NOFOLLOW`
  (`src/store/connection_guards.rs:94-102`); ATTACH/DETACH denied by authorizer
  on both connections (`:118-131`); `load_extension` off; two-arg
  `fts3_tokenizer` refused; `readfile`/`writefile` not compiled; VACUUM INTO,
  writable_schema, data-modifying CTEs and spliced statements refused by the
  `SELECT * FROM (...)` wrapper (`src/tools/query_exec.rs:149`,
  `query_safety_tests.rs:70-240`). No write attempt touched disk.
- `line_count`/`line_range` refuse >32 MiB input, type-check strictly, cannot
  panic on extreme bounds (`src/tools/sql_functions.rs:73-93`).
- CAS publish under IMMEDIATE transaction, current-revision flip last
  (`src/sync/revision.rs:139-140,249-262`); one deadline spans all retries
  (`src/sync/publish_cas.rs:84`); semaphore permit moved into the closure
  (`src/server_runner.rs:139,148-149`).
- Discovery does not follow symlinks (`src/sync/discovery.rs:136,290-306`);
  non-UTF-8 paths are counted (`:169,205`); capped reads use `take(max+1)`
  (`src/util/capped_read.rs:68-76`).
- `project_not_enabled` carries an exact remedy (`src/error_codes.rs:99-102`);
  `covers: rust_only|all_languages` makes a zero machine-readable.
- Findings go `stale` after the file changes; session purge works as designed.
