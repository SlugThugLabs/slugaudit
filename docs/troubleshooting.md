# Troubleshooting

Tool errors carry `data.code` (a stable string) and `data.remedy` (the next action). Branch on the code. Follow the remedy. Do not retry an unchanged request that failed with a permanent limit.

First check: call `health` with the project `path`. It never fails on a project that is not enabled, and it tells you the next step.

## Error codes

| Code | Meaning | What to do |
|---|---|---|
| `project_not_enabled` | No folder above the path has a `.planning/slugaudit/` directory (a symlink there does not count). | Call `project_control` with `{"action":"on","path":"<project directory>"}`. |
| `sync_budget_exhausted` | The freshness check ran past `max_sync_wall_clock`. On a project too big for the budget this is permanent. | Set `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`, or use the `audit` profile (1200 s) instead of `lean` (120 s). |
| `stale_revision` | The index changed under this call (another writer, or a file changed while it was being read). | Retry the call. It re-syncs. |
| `unknown_mechanic` | `leads` got a mechanic id that does not exist. | Call `leads` with only `path` to list the ids, then retry with one. |
| `sql_too_large` | The SQL is longer than `max_query_sql_bytes`. | Shorten it, or raise `SLUGAUDIT_MAX_QUERY_SQL_BYTES`. |
| `sql_error` | SQLite could not compile or run the statement. | Fix the statement from the message. List tables with `SELECT name FROM sqlite_master WHERE type='table'`, columns with `SELECT name FROM pragma_table_info('<table>')`, or read `slugaudit://schema.sql`. |
| `file_not_indexed` | The file has no stored text: never found, skipped (oversize, unreadable, not UTF-8) or binary. | Check with `SELECT path, file_kind, skip_reason FROM files WHERE path LIKE '...'`. See [how it works](how-it-works.md). |
| `mechanic_budget_exhausted` | A registered mechanic's scan used the whole `max_structure_execution_time`. Permanent for the same scan. | Remove the entry (`mechanic` `remove`), narrow `leads` `pattern`, or raise `SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS`. |
| `unknown_language` | A project-wide `structure` call named a language no indexed file has. | Use one of the languages the message lists, spelled as stored. `report` lists them under `languages`. |
| `unknown_cursor` | The cursor is malformed, belongs to another tool, or is from before a server restart. | Copy it exactly, or repeat the request without a cursor. |
| `result_expired` | The stored result was reclaimed to make room. | Repeat the request without a cursor. |
| `result_store_full` | All space is held by results still being paged. | Finish or abandon open paged reads, or narrow the request. |
| `result_too_large` | One result is bigger than the whole result store. | Add `LIMIT` or filters. |
| `result_store_unavailable` | The result store cannot work: its disk is full or unwritable, or the platform lacks what it needs. Other tools keep working. | Free disk space or check permissions on `.planning/slugaudit/results`. |
| `paging_parameter_removed` | The request had `offset` or `expected_revision`. | Page with `cursor` from `page.cursor`, or omit it for a new request. |

Unknown resource URIs return `resource_not_found`.

## Errors without a code

These come as plain messages.

| Message starts with or contains | Cause | What to do |
|---|---|---|
| `limits could not be resolved: ...` | The global `~/.slugthug/slugaudit/config.json` is unreadable, invalid JSON, has an unknown key, or is over 1 MiB. The message names the file. | Fix or remove the file. See [configuration](configuration.md). |
| `the SLUGAUDIT_* environment is invalid` or a startup `Error:` naming a variable | A `SLUGAUDIT_*` variable (or `RUST_LOG`) is set to a bad value, or is `0`. | Fix it and restart the server. |
| `import would load N bytes across sampled files, exceeding the L-byte ceiling` | The project is bigger than `max_total_import_bytes`. | Raise `max_total_import_bytes` (the global `config.json` or `SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES`), or use the `audit` profile. |
| `failed to walk project files` or `failed to apply the project's ignore rules` | The walk could not list a folder, or an ignore file could not be applied. SlugAudit refuses to index a partial set. | Fix the permissions or the ignore file named in the log. |
| `offset and expected_revision were removed` | Old paging parameters. | Use `cursor`. |
| `path exceeds 32768 bytes` and similar | Input over its size limit. | Shorten the value. |
| `SQLite did not enable WAL journal mode` | The project's filesystem does not support SQLite WAL. | Use a project location on a local filesystem. |
| A refusal about a symlink at the database path | `.planning/slugaudit/project.db` is a symlink. SlugAudit will not write through it. | Replace it with a regular file or remove it. |

## "Project not enabled"

You see `project_not_enabled` when the path is not inside an enabled project.

1. Call `project_control` with `"action": "on"` and the project root as `path`.
2. If it was enabled before, check that the directory `.planning/slugaudit/` exists. If the directory was deleted, the project is off. A missing `project.db` alone does not turn it off: the next call rebuilds it.
3. If the folder was moved, copied or unpacked from an archive, the old database is thrown away and rebuilt on the next call.

## The database was damaged

You do not need to do anything. If the database is corrupt, has an incompatible version, or was built for another project root, SlugAudit deletes it and rebuilds it from your source in the same call. That call's response carries `database_discard`:

```json
{ "cause": "the project database is corrupt or outdated: ..." }
```

Nothing is lost: this session's findings live in their own database, `findings-<id>.db`, which a rebuild never touches. If you want to rebuild by hand, call `project_control` with `off`, then `on`. That also deletes the session's findings.

## Offline: grammar downloads fail

Languages other than Rust need a grammar downloaded on first use. Offline:

- The first failed fetch can block a call for about 60 seconds.
- Affected files are stored but have no evidence, and `parser_availability` is `LoadFailed`.
- SlugAudit does not try to download again for 10 minutes, or until the server restarts.
- Text search with `query` on `content` keeps working.

Details are in [languages](languages.md).

## Results look incomplete

Check these before you trust a zero or an absence:

- `leads.files_not_parsed` above 0: every count is a floor.
- `leads.mechanic_counts[].covers`: a language missing there was not checked.
- `report.completeness.index_partial` is `true`: some files are skipped or have unusable names.
- `report.completeness.skipped_by_reason`: why files were skipped.
- `report.parser_failure_files`: files with no evidence and the reason.
- `page.complete` is `false`: more pages remain. Use `page.cursor`.
- `page.completeness` is `partial`: the scan stopped early. Start a new request from `resume_after_file`.

## Slow calls

See [performance](performance.md). The cost is hashing every file on every call. A cold disk or too little RAM to cache the project makes it slower.

## Logs

SlugAudit writes logs to standard error. Standard output carries the MCP messages and is never used for logs. Where standard error ends up depends on your agent. Some show it in an MCP log view, some discard it. SlugAudit does not write a log file.

The default level shows warnings plus SlugAudit's own info lines. Set `RUST_LOG` to change it and `SLUGAUDIT_LOG_FORMAT=json` for JSON lines. See [configuration](configuration.md).

The code is written to keep your source text, SQL and finding text out of the logs, and a test checks this for the tools. Logs can contain file paths.

## Setup commands

| Problem | Cause | What to do |
|---|---|---|
| `connect` says the agent is unknown | The id is not in the list | Run `slugaudit connect` with no name to see detected agents, or see [getting started](getting-started.md) for the ids. |
| `connect goose` or `connect crush` refuses | Those clients need a manual config edit | Paste the lines it prints. |
| `connect` times out | The agent's own CLI did not finish in 15 s (for example it waited at a login prompt) | Run the agent's CLI once by hand, then retry. |
| `update` stops with an unsupported platform error | The only published release is Linux x86_64 | Build from source: `cargo install --git https://github.com/SlugThugLabs/slugaudit.git --bin slugaudit`. |
| `update` fails with a checksum error | The download did not match `SHA256SUMS`. Nothing was replaced. | Retry later. |
| A new version is worse | | Copy `slugaudit.prev` over `slugaudit` in `~/.slugthug/slugaudit/`. |
| Gemini shows the server as Disabled | Gemini disables servers until the folder is trusted | Trust the folder in Gemini. |
