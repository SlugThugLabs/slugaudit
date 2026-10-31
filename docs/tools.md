# Tools and resources

SlugAudit exposes 10 MCP tools and 10 MCP resources. This page lists each tool with its parameters, key response fields, and an example call.

Facts that hold for every tool:

- Line numbers are 1-based and inclusive.
- `path` is any path inside an enabled project (the root, a folder, or a file). It selects which project's database to use.
- Every tool except `health` brings the index up to date with the disk before it answers (`project_control` with `on` indexes the project; `off` does not index). A call that continues a `cursor` does not sync again. See [how it works](how-it-works.md).
- A project that is not enabled fails with `project_not_enabled`. See [troubleshooting](troubleshooting.md).
- Errors carry `data.code` and `data.remedy`. Branch on the code. Follow the remedy.
- A response that rebuilt a damaged database carries `database_discard` (see below).

| Tool | Purpose | Changes anything? |
|---|---|---|
| [`report`](#report) | Project snapshot | no |
| [`leads`](#leads) | Counts and rows of mechanical facts | no |
| [`mechanic`](#mechanic) | Add, remove, list your own counters | session memory, optionally `config.json` |
| [`query`](#query) | Read-only SQL over the index | no |
| [`structure`](#structure) | Tree-sitter pattern search | no |
| [`finding`](#finding) | Record a conclusion you reviewed | writes to the database |
| [`finding_read`](#finding_read) | Read this session's findings | no |
| [`finding_sessions`](#finding_sessions) | List or adopt earlier sessions' findings | writes to the database |
| [`project_control`](#project_control) | Enable or disable a project | creates or removes the database |
| [`health`](#health) | Server and database state | no |

## Paging

`leads`, `query`, `structure`, `finding_read` and `mechanic` (action `list`) return a `page` object:

| Field | Meaning |
|---|---|
| `returned` | Items in this page. |
| `total` | Items the whole request selects. `null` when it cannot be counted (always for `structure`; for `query` when counting ran out of budget). |
| `complete` | `true` when nothing remains. |
| `cursor` | Token for the next page. Absent when `complete` is true. Does not survive a server restart. |
| `completeness` | Absent, `"complete"`, or `partial` (see below). |
| `page_bytes` | The byte budget that applied. |
| `bytes_used` | Serialized JSON bytes of this page. |
| `limited_by` | What cut the page: `none`, `limit`, `page_bytes`, `ceiling`, `match_cap`, `scan_window`, `time_budget`. |
| `revision_id` | The index revision the answer was read from. |

How to page:

1. Make the first call. The whole answer is computed once and stored.
2. While `page.cursor` is present, call again with only `cursor` (plus `page_bytes`, and `limit` on `leads`). No `path` is needed.
3. Pages come from the stored copy. Edits made in between never skip or repeat a row. A cursor call does not re-sync.

Page size:

- Pages are cut by bytes, not by row count. The default is 64 KiB (`default_page_bytes`).
- Pass `page_bytes` to ask for more, up to the ceiling `max_query_response_bytes` (32 MiB on `lean`, 256 MiB on `audit`, host-dependent on `adaptive`). A larger value is clamped and `limited_by` says `ceiling`.
- The first item of a page is always returned, even if it alone is bigger than the page.

`partial` completeness: a `structure` search or a custom-mechanic scan stopped early. The object has `limited_by`, `resume_after_file`, `files_scanned`, `files_total`. Resuming is a new request that syncs again (for `structure`, pass `start_after_file`).

Paging errors:

| Code | Meaning | What to do |
|---|---|---|
| `unknown_cursor` | Malformed, from another tool, or from before a restart | Repeat the request without a cursor |
| `result_expired` | The stored result was reclaimed | Repeat the request without a cursor |
| `result_store_full` | Open paged reads use all the space | Finish them, or narrow the request |
| `result_too_large` | One result is bigger than the whole store | Add `LIMIT` or filters |
| `result_store_unavailable` | The result store cannot work here (disk full or unwritable) | Other tools keep working |
| `paging_parameter_removed` | The request had `offset` or `expected_revision` | Use `cursor` |

The result store budget is 4 times `max_query_response_bytes`. An unfinished result is protected from reclaim for 900 seconds after its last page.

## `notices`

`report`, `leads` and `structure` return a `notices` array of strings. It has one line per code language that is not tested. Only Rust is tested. The array is empty when there is nothing to say. See [languages](languages.md) for the exact text.

## `database_discard`

Present only on the call that threw away and rebuilt the database. It has `findings_discarded` (count; `null` if the database was too damaged to count) and `cause`. Findings live in the database, so a rebuild loses them. Check this field.

---

## report

Project snapshot. Use it first.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string, required | Any path inside the project. |

Key response fields:

| Field | Meaning |
|---|---|
| `revision_id` | Index revision (`rev-N`). |
| `file_count` | Rows in `files`, including binary and skipped files. |
| `completeness` | Whether the counts are the whole truth (below). |
| `languages` | `[{language, file_count, tested}]`. Files with no detected language are not listed. |
| `notices` | Untested-language lines. |
| `evidence_counts` | `[{kind, count}]` for Structure, Import, Export, Comment, Docstring, Symbol, Diagnostic, Call. |
| `parser_failure_count`, `parser_failure_files`, `parser_failure_files_total` | Files whose parser was unavailable or failed. The list holds up to 20 files. |
| `open_finding_count` | Findings with status `current`. |
| `import_resolution` | `[{kind, count}]`, kinds `Resolved`, `Unresolved`, `External`. |
| `dependency_edges` | `outbound_edge_count`, `inbound_edge_count`, `outbound_file_count`, `inbound_file_count`. |
| `diagnostic_count` | Diagnostic evidence rows. |
| `unsupported_language_unresolved_count` | Unresolved edges from import syntax that is not modeled. |
| `credential_pattern_file_count` | Files whose path looks like a credential file (`.env`, `*.pem`, `*.key`, `id_rsa`, ...). |
| `mechanics` | Every mechanic (`entries`), the ones that failed to load (`invalid`), and a one-time `note`. |
| `mechanics_invalid` | Count of failed entries. |

`completeness` fields:

| Field | Meaning |
|---|---|
| `file_kind_counts` | Rows by `indexed`, `binary`, `skipped`. |
| `file_class_counts` | Rows by `authored`, `generated`, `vendored` (a path heuristic). |
| `skipped_file_count`, `skipped_by_reason` | Skipped files and why: `oversize`, `unreadable`, `stat_failed`, `not_utf8`. |
| `extraction_completeness_counts` | Rows by `Full`, `Partial`, `ContentOnly`, `Unavailable`. |
| `index_partial` | `true` when some discovered file has no indexed row. |
| `unrepresentable_path_count` | Files whose name is not valid UTF-8. They cannot appear in the index at all. |
| `indexed_files_without_evidence`, `prose_files_not_parsed` | Text is stored but nothing structural was extracted. |
| `oversize_for_parse_file_count`, `parse_timeout_file_count` | Files stored but not parsed (too big, or the parse timed out). |
| `evidence_capped_file_count` | Files whose evidence was cut by a cap. Their counts are a floor. |

Example:

```json
{ "path": "/home/you/code/myproject" }
```

## leads

Counts and rows of mechanical facts. Nothing is ranked, scored or judged. What matters is your call.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string | Required unless `cursor` is given. |
| `mechanic` | string | One mechanic id. Omit to get counts only. |
| `pattern` | string | Path filter. A GLOB, or a substring when it has no `*` or `?`. Narrows counts and rows. |
| `limit` | integer | Max rows in the page. Omit for everything that fits. `0` counts as `1`. |
| `mode` | `full` or `compact` | `compact` returns `mechanic`, `file`, `line` only. With no `mechanic`, it lists every mechanic's rows in one response. |
| `page_bytes` | integer | Page size in bytes. |
| `cursor` | string | Previous `page.cursor`. Other request fields are ignored. |

Built-in mechanic ids: `unwrap_calls`, `expect_calls`, `panic_calls`, `todo_calls`, `assert_calls`, `diagnostics`, `comment_markers` (TODO, FIXME, HACK, XXX), `credential_files`. Your registered mechanics (see `mechanic`) are listed first.

Key response fields:

| Field | Meaning |
|---|---|
| `mechanic_counts` | `[{mechanic, source, note, count, files, covers}]`. Counts are complete, never paged. `source` is `session`, `project_config` or `builtin`. `covers` lists the languages the count read. A language missing from `covers` was not checked, so a zero there means nothing. |
| `rows` | `full`: `{file, line, detail, detail_clipped}`. `compact`: `{mechanic, file, line}`. `line` is `0` when there is no span. `detail` is cut at 200 characters. |
| `files_total`, `files_covered` | File denominators. `files_total - files_covered` is what is left. |
| `files_not_parsed` | Indexed files whose grammar did not run or finish. While above 0, every count is a floor. `report` lists the files. |
| `page_file_counts` | Rows per file in this page. |
| `oversized_row` | `true` when the first row alone is bigger than the page. |
| `mechanics_invalid` | Registered entries that failed to load. |
| `notices` | Untested-language lines for the languages the counts cover. |

The call-site mechanics (`unwrap_calls` and the like) match Rust callee names. They read calls in rust, python, javascript, typescript, tsx, go, java and c.

Example, counts first, then rows:

```json
{ "path": "/home/you/code/myproject" }
```
```json
{ "path": "/home/you/code/myproject", "mechanic": "unwrap_calls", "pattern": "src/auth/*", "mode": "compact" }
```

## mechanic

Register, remove or list your own counters. A mechanic is a Tree-sitter query with exactly one capture. Each distinct captured node is one row.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `action` | `add`, `remove`, `list` | Required. |
| `path` | string | Required unless `cursor` is given. |
| `id` | string | `add`, `remove`. 1-64 characters of `a-z`, `0-9`, `_`. Never a built-in's id or one already in use. |
| `index` | integer | `remove` only, instead of `id`: removes a persisted entry that failed to load (listed in `invalid`). |
| `language` | string | `add`. The lowercase name the index stores (`python`, `rust`, `javascript`, `tsx`). Not a pack alias such as `shell` (use `bash`). Prose is refused. |
| `query` | string | `add`. Tree-sitter S-expression with exactly one capture. Up to 16384 bytes by default. A query that does not compile is refused with the compiler's message. |
| `note` | string | `add`. 1 to 1024 bytes. What the entry counts, in your words. |
| `persist` | boolean | `add`. `true` writes the entry to `.planning/slugaudit/config.json`. Default `false`: kept in server memory until the server exits. |
| `limit`, `page_bytes` | integer | `add`: size of the first page. `list`: `page_bytes` is the page size. |
| `cursor` | string | `list` only. |

Limits: 64 session entries and 64 persisted entries.

Key response fields: `action`, `id`, `source`, `mechanics` (the catalogue), `invalid`, `mechanics_invalid`. On `add`, `first_count` is the full `leads` response for the new id. On `list`, `page` is present.

Example:

```json
{
  "path": "/home/you/code/myproject",
  "action": "add",
  "id": "eval_calls",
  "language": "python",
  "query": "(call function: (identifier) @callee (#eq? @callee \"eval\"))",
  "note": "Calls to eval()",
  "persist": false
}
```

## query

Read-only SQL over the index. One `SELECT` or `WITH` statement per call.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string | Required unless `cursor` is given. |
| `sql` | string | The statement. Required unless `cursor` is given. |
| `page_bytes` | integer | Page size in bytes. |
| `cursor` | string | Previous `page.cursor`. |

Tables:

| Table | Columns |
|---|---|
| `files` | `id`, `path`, `language`, `byte_len`, `content`, plus `file_kind`, `skip_reason`, `file_class`, `content_hash`, `parser_availability`, `parse_outcome`, `parse_error_reason`, `extraction_completeness`, ... |
| `evidence` | `id`, `file_id`, `kind`, `start_line`, `end_line`, `payload` (JSON). Kinds: Structure, Import, Export, Comment, Docstring, Symbol, Diagnostic, Call. |
| `dependency_edges` | `from_file_id`, `to_file_id`, `raw_import_text`, `resolution_kind` (`Resolved`, `Unresolved`, `External`), `confidence`, `syntax_unmodeled` |

The full schema is the resource `slugaudit://schema.sql`. Join `evidence` to `files` on `file_id`. Read payload keys with `json_extract`, not `LIKE`. Payload keys are listed in `slugaudit://queries/evidence_payload_keys.md`.

Helpers: `line_count(text)` and `line_range(text, start, end)` read a file's content by line.

Rules:

- Only `SELECT` or `WITH`. PRAGMA, EXPLAIN, writes and DDL are refused. The introspection functions `pragma_table_info`, `pragma_table_xinfo`, `pragma_table_list`, `pragma_index_list`, `pragma_index_info`, `pragma_index_xinfo`, `pragma_foreign_key_list` work.
- SQL longer than `max_query_sql_bytes` fails with `sql_too_large` (50,000 bytes on `lean`, 200,000 otherwise).
- SQLite compile or run errors fail with `sql_error`.
- A query that spends its VM-step or wall-clock budget fails as a whole and returns no cursor. Limits are 5,000,000 steps and 10 s on `lean`, 50,000,000 steps and 60 s otherwise.
- Result column names must be unique. Alias duplicates.
- Row order is the query's own. No `ORDER BY` is added.
- Add `LIMIT` when exploring.

Extra response fields:

| Field | Meaning |
|---|---|
| `rows` | One JSON object per row. |
| `clipped_cells` | TEXT or BLOB cells cut from the first row because the row alone was bigger than the page. Each has `row`, `column`, `returned`, `total`, `next_start`, `continue_with` (a ready `substr` expression). Nothing is lost. |
| `non_finite_cells` | REAL cells shown as `null` because JSON cannot carry Infinity or NaN. A `null` not listed here is SQL NULL. |

Example, read a file:

```json
{ "path": "/home/you/code/myproject", "sql": "SELECT content FROM files WHERE path = 'src/main.rs'" }
```

Example, find a symbol:

```json
{
  "path": "/home/you/code/myproject",
  "sql": "SELECT f.path, e.start_line FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND json_extract(e.payload, '$.name') = 'configure' LIMIT 20"
}
```

## structure

Tree-sitter pattern search on one file or on every file of one language.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string | Required without `cursor`. |
| `query` | string | Required. Tree-sitter S-expression with at least one `@name` capture. Up to 16384 bytes by default. |
| `file` | string | Project-relative path of one file. |
| `language` | string | Grammar name (`rust`, `python`, ...). Required when `file` is omitted. |
| `pattern` | string | Path filter for multi-file searches, for example `src/**/*.rs`. |
| `full_text` | boolean | Return the full matched text (up to 1,000,000 bytes). Default `true` for one file, `false` for many files. |
| `start_after_file` | string | Multi-file only: scan only files whose path sorts after this one. |
| `page_bytes`, `cursor` | | As in paging. |

Response fields:

| Field | Meaning |
|---|---|
| `language` | The grammar used. |
| `notices` | One line when the language is untested. |
| `matches` | `{file, capture_name, node_kind, start_byte, end_byte, start_line, start_column, end_line, end_column, text, text_truncated, extraction_failed}` |
| `files_not_scanned`, `bytes_not_scanned` | Multi-file only: files past the 64 MiB scan window. |
| `next_start_after_file` | Value for `start_after_file` in the next request. |
| `unparsed_files` | Files the parser produced no tree for. |
| `files_over_parse_limit` | Files larger than the parse limit (2 MiB by default). They are skipped here but searchable with `query`. |
| `parse_timed_out_file` | The file whose parse ran out of time. |

A project-wide search for a language no indexed file has fails with `unknown_language`. The message lists the languages that are indexed. A single file that is not indexed with content fails with `file_not_indexed`. A page holds at most `max_structure_matches` matches (2,000 on `lean`, 20,000 otherwise).

Example:

```json
{
  "path": "/home/you/code/myproject",
  "language": "rust",
  "pattern": "src/**/*.rs",
  "query": "(function_item name: (identifier) @name)"
}
```

## finding

Record a conclusion you have reviewed. SlugAudit never writes findings itself.

Parameters (all required):

| Name | Type | Notes |
|---|---|---|
| `path` | string | Any path inside the project. |
| `file` | string | Project-relative path. The file must be indexed with content. |
| `line_start`, `line_end` | integer | 1-based, inclusive. `line_end` must be at least `line_start` and not past the end of the file. |
| `severity` | string | Your scale. Up to 100 characters. |
| `category` | string | Your label. Up to 100 characters. |
| `title` | string | Up to 200 characters. |
| `description` | string | Up to 10,000 characters. |

A file that is not indexed with content (binary, skipped, never found) fails with `file_not_indexed`. The finding is tied to the file's content hash. When the file changes, the finding becomes `stale`.

Response: `id`, `revision_id`, `source_hash`, `status` (`current`).

Example:

```json
{
  "path": "/home/you/code/myproject",
  "file": "src/auth.rs",
  "line_start": 42, "line_end": 48,
  "severity": "high", "category": "correctness",
  "title": "Token compared with ==",
  "description": "The token is compared with a non-constant-time equality check."
}
```

## finding_read

Read the findings written by this session. It never returns another session's findings.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string | Required unless `cursor` is given. |
| `file` | string | Only findings for this file. |
| `cursor` | string | Previous `page.cursor`. |

Response: `page` and `findings`, each `{id, path, line_start, line_end, severity, category, title, description, detail_clipped, status, source_hash}`, ordered by id. `status` is `current` or `stale`. `description` may be clipped to fit the page (`detail_clipped`).

Example: `{ "path": "/home/you/code/myproject" }`

## finding_sessions

A server restart is a new session. `finding_read` shows only the current session's findings. Earlier sessions' findings are kept and marked `stale`. Use this tool to see them or take them over.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string | Required. |
| `action` | `list` or `adopt` | Required. |
| `session_id` | string | Required for `adopt`, refused for `list`. Not the current session's own id. |

Response: `sessions` (`[{session_id, started_at_unix, findings}]`, newest first, at most 100), `total_sessions`, `adopted` (count, for `adopt`). Adopted findings read `current` if their file is unchanged, else `stale`. Nothing is deleted.

Example: `{ "path": "/home/you/code/myproject", "action": "list" }`

## project_control

Turn a project on or off.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `action` | `on` or `off` | Required. |
| `path` | string | Project root. Defaults to the server's current directory. |
| `profile` | `adaptive`, `audit`, `lean` | Optional. Saved to `config.json` on `on`. |

Response:

| Field | Meaning |
|---|---|
| `status` | `enabled`, `disabled`, or `already_disabled` (nothing to remove). |
| `path` | Absolute project root. |
| `import` | On `on`: `added`, `modified`, `deleted`, `unchanged`, `skipped`, `revision_id`. |
| `gitignore` | On `on`: `status` (`added`, `already_ignored`, `not_written`), `path`, `error`. |
| `removed`, `config_kept` | On `off`: the paths removed, and whether `config.json` was kept. |
| `database_discard` | Present when `on` found a damaged database and rebuilt it. |

`off` removes the database, its SQLite sidecars, the `results` directory and the config lock. It keeps `config.json`. It removes the `.planning/slugaudit` directory only if nothing else is left in it. `on` brings back the settings in `config.json`. If the project folder was moved, copied or unpacked elsewhere, the old database is thrown away and rebuilt on the next call.

Example: `{ "action": "on", "path": "/home/you/code/myproject" }`

## health

What state the server is in. Read-only. It never syncs, never changes anything, and does not fail on a project that is not enabled.

Parameters:

| Name | Type | Notes |
|---|---|---|
| `path` | string | Optional. Without it, no project is selected. |
| `verbose` | boolean | Default `false`. Adds `detail`. |

Response:

| Field | Meaning |
|---|---|
| `phase` | `importing` (no revision yet), `steady_state`, `disabled` (path is not an enabled project), `no_active_project` (no path given), `unknown` (database exists but cannot be read). |
| `remedy` | The call to make when no project is served. `null` otherwise. |
| `database` | `{revision_id, file_count}`. `null` when never synced or unreadable. |
| `detail` | With `verbose`: parser pack version, skipped file count, whether the database read failed, process counters, host memory, mechanic scan cache, result store. |

Example: `{ "path": "/home/you/code/myproject", "verbose": true }`

---

## Resources

Resources are read on demand with `resources/read`. They cost nothing until read.

| URI | What it is |
|---|---|
| `slugaudit://queries/README.md` | Query cookbook |
| `slugaudit://queries/evidence_payload_keys.md` | Every `evidence.payload` key, its JSON type, and its kind |
| `slugaudit://queries/find_unhandled_panics.sql` | Rust call sites whose callee is `unwrap`, `expect` or `panic` |
| `slugaudit://queries/auth_attack_surface.sql` | Functions, methods and calls whose name mentions auth, tokens, sessions or secrets |
| `slugaudit://queries/syntax_diagnostics.sql` | Syntax errors and parser diagnostics per file |
| `slugaudit://queries/rust_functions.scm` | Tree-sitter pattern for Rust functions |
| `slugaudit://queries/blast_radius_outbound.sql` | Per-file outbound dependency counts by resolution kind |
| `slugaudit://queries/blast_radius_inbound.sql` | Per-file inbound dependency counts |
| `slugaudit://queries/blast_radius_closure.sql` | Transitive import closure of one file |
| `slugaudit://schema.sql` | The SQLite schema of the index |

An unknown URI returns `resource_not_found`.
