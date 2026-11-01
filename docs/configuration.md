# Configuration

Configuration is being simplified; this page documents the current version.

SlugAudit works with no configuration. You can change its limits in three ways:

1. A profile: `lean`, `audit` or `adaptive`.
2. Environment variables (`SLUGAUDIT_*`), read once when the server starts.
3. A per-project file, `.planning/slugaudit/config.json`, read on every call.

## Profiles

| Profile | Meant for |
|---|---|
| `lean` | Everyday coding assistant use. Small, fast budgets. |
| `audit` | Deep audits. Large files, long time budgets. |
| `adaptive` | The default. Limits that scale with the machine's RAM. |

Choose a profile with the environment variable `SLUGAUDIT_PROFILE` (case-insensitive: `lean`, `audit`, `adaptive`), with the `profile` key in `config.json`, or with the `profile` parameter of `project_control` `on` (which writes the key for you). An unknown value in the environment stops the server at startup, with an error that names the variable.

If the machine's memory cannot be detected, `adaptive` cannot be sized. The server then needs an explicit profile. Without one it stops at startup.

### Default limits by profile

| Limit | `lean` | `audit` | `adaptive` |
|---|---|---|---|
| `max_file_bytes` (largest file stored) | 32 MiB | 256 MiB | RAM / 10, at least 256 MiB, at most 1,000,000,000 |
| `max_parse_bytes` (largest file parsed) | 2 MiB | 2 MiB | 2 MiB |
| `max_parse_wall_clock_ms` (per-file parse time) | 5000 | 5000 | 5000 |
| `max_total_import_bytes` (all files in one call) | 2 GiB | 32 GiB | available RAM / 2, at least 4 GiB |
| `max_query_response_bytes` (page ceiling) | 32 MiB | 256 MiB | same as `max_file_bytes` |
| `default_page_bytes` | 64 KiB | 64 KiB | 64 KiB |
| `max_query_sql_bytes` | 50,000 | 200,000 | 200,000 |
| `max_query_vm_steps` | 5,000,000 | 50,000,000 | 50,000,000 |
| `max_query_wall_clock` | 10 s | 60 s | 60 s |
| `max_query_value_bytes` | 32 MiB | 256 MiB | same as `max_file_bytes` |
| `max_structure_query_bytes` | 16,384 | 16,384 | 16,384 |
| `max_structure_matches` | 2,000 | 20,000 | 20,000 |
| `max_structure_execution_time` | 10 s | 60 s | 60 s |
| `max_sync_wall_clock` | 120 s | 1200 s | 1200 s |
| `max_result_store_bytes` | 128 MiB | 1 GiB | 4 x `max_query_response_bytes` |
| `result_active_grace_secs` | 900 | 900 | 900 |

`max_parse_bytes` is never allowed to exceed `max_file_bytes`. If it does, it is lowered to match and a warning is logged.

No value can be stored if it exceeds 1,000,000,000 bytes (the largest value the bundled SQLite can hold). `max_file_bytes` and `max_query_value_bytes` are clamped to that, with a warning.

## `config.json`

Path: `<project>/.planning/slugaudit/config.json`. It is optional. SlugAudit creates it only when you persist something (`project_control` with `profile`, or `mechanic` with `persist: true`). `project_control` `off` deletes it along with the rest of `.planning/slugaudit/`.

Example:

```json
{
  "profile": "lean",
  "max_file_bytes": 67108864,
  "max_parse_bytes": 1048576
}
```

Keys:

| Key | Type | What it sets |
|---|---|---|
| `profile` | `"lean"`, `"audit"`, `"adaptive"` | The base set of limits for this project. |
| `max_file_bytes` | integer | Largest file stored. |
| `max_parse_bytes` | integer | Largest file parsed. |
| `max_parse_wall_clock_ms` | integer | Parse time allowed for one file, in milliseconds. |
| `max_structure_query_bytes` | integer | Longest `structure` or `mechanic` query. |
| `max_total_import_bytes` | integer | Total bytes of all files read in one call. |
| `default_page_bytes` | integer | Default page size of `query` and `leads`. |
| `max_result_store_bytes` | integer | Disk budget for stored paged results. |
| `result_active_grace_secs` | integer | Seconds an unfinished paged result is protected from reclaim. |
| `mechanics` | object | Your persisted mechanics, written by `mechanic` with `persist: true`. Do not hand-edit unless you know the format: `{"version": 1, "entries": [{"id", "language", "query", "note"}]}`. At most 64 entries. |

Rules:

- Any other key is an error. A misspelled key is reported, not ignored.
- A value of `0` is ignored and a warning is logged.
- Order of precedence: the base is the environment-selected limits; if `profile` is set in `config.json`, the whole base is replaced by that profile's limits (this drops any `SLUGAUDIT_*` limit overrides); then each key in `config.json` is applied on top.
- The file is read on every call, so changes take effect on the next call. No restart is needed.
- A file that cannot be read, is not valid JSON, contains an unknown key, has a directory at its path, or is larger than 1 MiB fails the call with `project config is unusable: ...; fix or remove the file and retry`. The message names the file.

## Environment variables

Set these in the environment of the server process. How you do that depends on your agent: most MCP clients have an `env` setting for each server. The values are read once when the server starts. Restart the server to change them.

A variable that is set but cannot be parsed stops the server at startup, with an error that names the variable (not its value). A `0` is also an error. A variable that is not set changes nothing. A misspelled variable name is not detected.

Limits (the name is `SLUGAUDIT_` plus the upper-case limit name):

| Variable | Type | Also in `config.json` |
|---|---|---|
| `SLUGAUDIT_PROFILE` | `lean`, `audit`, `adaptive` | yes (`profile`) |
| `SLUGAUDIT_MAX_FILE_BYTES` | bytes | yes |
| `SLUGAUDIT_MAX_PARSE_BYTES` | bytes | yes |
| `SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS` | milliseconds | yes |
| `SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES` | bytes | yes |
| `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES` | bytes | no |
| `SLUGAUDIT_DEFAULT_PAGE_BYTES` | bytes | yes |
| `SLUGAUDIT_MAX_QUERY_SQL_BYTES` | bytes | no |
| `SLUGAUDIT_MAX_QUERY_VM_STEPS` | steps | no |
| `SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS` | whole seconds | no |
| `SLUGAUDIT_MAX_QUERY_VALUE_BYTES` | bytes | no |
| `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES` | bytes | yes |
| `SLUGAUDIT_MAX_STRUCTURE_MATCHES` | count | no |
| `SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS` | whole seconds | no |
| `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` | whole seconds | no |
| `SLUGAUDIT_MAX_RESULT_STORE_BYTES` | bytes | yes |
| `SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS` | whole seconds | yes |

If you do not set `SLUGAUDIT_MAX_RESULT_STORE_BYTES`, the result store budget is 4 times the effective `max_query_response_bytes`, including its override.

Logging:

| Variable | Effect |
|---|---|
| `RUST_LOG` | Log filter (tracing syntax such as `info` or `slugaudit_mcp_rust=debug`). Default: `warn,slugaudit_mcp_rust=info,rmcp=error`. An invalid value stops startup. |
| `SLUGAUDIT_LOG_FORMAT` | `json` for JSON lines. Anything else is plain text. |

Logs go to standard error. Standard output is the MCP transport.

Install location:

| Variable | Effect |
|---|---|
| `SLUGTHUG_HOME` | Base directory for the installed binary. Default `~/.slugthug`. The binary goes in `<base>/slugaudit/`. |
| `SLUGTHUG_CONFIG_HOME` | Home directory used to find and write each agent's own config files. Default `$HOME`. |

## Tool parameters are the other knobs

`page_bytes`, `limit` and `pattern` on individual calls change one answer, not the configuration. See [tools](tools.md).
