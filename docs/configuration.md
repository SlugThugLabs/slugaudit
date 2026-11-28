# Configuration

SlugAudit works with no configuration. There is no per-repo config file. You can adjust limits in three ways:

1. A human-owned global file, `~/.slugthug/slugaudit/config.json` (optional, read-only).
2. Environment variables (`SLUGAUDIT_*`).
3. The session `profile` your agent passes to `project_control` `"action": "on"`.

Everything set through tools (findings, mechanics, the session profile) lasts one server process and is wiped when SlugAudit restarts.

## Order of precedence

1. **Profile selection** (picks the baseline limits):
   - Session `profile` passed to `project_control` `on` (highest), then
   - `SLUGAUDIT_PROFILE`, then
   - `"profile"` in `~/.slugthug/slugaudit/config.json`, then
   - `adaptive` (default).
2. **Limit overrides** (applied on top of the selected profile's baseline, from lowest to highest):
   - Profile baseline (`limits_for_profile`), then
   - Limit keys in `~/.slugthug/slugaudit/config.json`, then
   - `SLUGAUDIT_*` limit environment variables (highest).

Choosing a session profile picks the baseline only; it never discards an explicit limit you set in `config.json` or `SLUGAUDIT_*`.

## Profiles

| Profile | Meant for |
|---|---|
| `lean` | Fast interactive assistant work with tighter time and size ceilings. |
| `audit` | Deep audits with larger file, response, and SQLite step budgets. |
| `adaptive` | Default. Scales memory-sensitive ceilings with detected host RAM. |

If host memory cannot be detected, `adaptive` cannot be sized: set `SLUGAUDIT_PROFILE` (or `"profile"` in `config.json`) to `lean` or `audit`, or startup stops with an explicit error.

### Default limits by profile

| Limit | `lean` | `audit` | `adaptive` |
|---|---|---|---|
| `max_file_bytes` (largest file stored) | 32 MiB | 256 MiB | RAM / 10, clamped between 32 MiB and 1,000,000,000 (`>= 2.56 GiB` hosts get at least 256 MiB) |
| `max_parse_bytes` (largest file parsed) | 2 MiB | 2 MiB | 2 MiB |
| `max_parse_wall_clock_ms` (per-file parse time) | 5,000 ms | 5,000 ms | 5,000 ms |
| `max_total_import_bytes` (sampled bytes in one sync) | 2 GiB | 32 GiB | available RAM / 2, at least 4 GiB on `>= 8 GiB` hosts (scaled to `max(max_file_bytes, RAM / 2)` on smaller hosts) |
| `max_query_response_bytes` (page ceiling) | 32 MiB | 256 MiB | same as `max_file_bytes` |
| `default_page_bytes` | 64 KiB | 64 KiB | 64 KiB |
| `max_query_sql_bytes` | 50,000 | 200,000 | 200,000 |
| `max_query_vm_steps` | 5,000,000 | 50,000,000 | 50,000,000 |
| `max_query_wall_clock` (`max_query_wall_clock_secs`) | 10 s | 60 s | 60 s |
| `max_query_value_bytes` | 32 MiB | 256 MiB | same as `max_file_bytes` |
| `max_structure_query_bytes` | 16,384 | 16,384 | 16,384 |
| `max_structure_matches` | 2,000 | 20,000 | 20,000 |
| `max_structure_execution_time` (`max_structure_execution_time_secs`) | 10 s | 60 s | 60 s |
| `max_sync_wall_clock` (`max_sync_wall_clock_secs`) | 30 s | 90 s | 90 s |
| `max_result_store_bytes` | 128 MiB | 1 GiB | 4 x `max_query_response_bytes` |
| `result_active_grace_secs` | 900 s | 900 s | 900 s |

Clamping rules:
- `max_parse_bytes` is clamped to `max_file_bytes` with a warning if configured higher.
- `max_file_bytes` and `max_query_value_bytes` are clamped to `1,000,000,000` bytes (bundled SQLite's maximum storable value size) with a warning.
- Unless `max_result_store_bytes` is explicitly overridden in `config.json` or `SLUGAUDIT_MAX_RESULT_STORE_BYTES`, the result-store budget scales automatically to `4 * max_query_response_bytes`.

## The global file (`~/.slugthug/slugaudit/config.json`)

Path: `~/.slugthug/slugaudit/config.json` (or `$SLUGTHUG_HOME/slugaudit/config.json` when `SLUGTHUG_HOME` is set). Optional and strictly read-only: MCP tools never write it, `project_control` `off` never touches it, and model-facing tool instructions do not mention it.

Example:

```json
{
  "profile": "lean",
  "max_file_bytes": 67108864,
  "max_parse_bytes": 1048576,
  "max_sync_wall_clock_secs": 60
}
```

Supported keys (`profile` plus all 16 limit keys; unknown keys are rejected):

| JSON key (`config.json`) | Environment variable | Unit / values |
|---|---|---|
| `profile` | `SLUGAUDIT_PROFILE` | `"lean"`, `"audit"`, `"adaptive"` |
| `max_file_bytes` | `SLUGAUDIT_MAX_FILE_BYTES` | bytes (> 0) |
| `max_parse_bytes` | `SLUGAUDIT_MAX_PARSE_BYTES` | bytes (> 0) |
| `max_parse_wall_clock_ms` | `SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS` | milliseconds (> 0) |
| `max_total_import_bytes` | `SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES` | bytes (> 0) |
| `max_query_response_bytes` | `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES` | bytes (> 0) |
| `default_page_bytes` | `SLUGAUDIT_DEFAULT_PAGE_BYTES` | bytes (> 0) |
| `max_query_sql_bytes` | `SLUGAUDIT_MAX_QUERY_SQL_BYTES` | UTF-8 bytes (> 0) |
| `max_query_vm_steps` | `SLUGAUDIT_MAX_QUERY_VM_STEPS` | SQLite VM steps (> 0) |
| `max_query_wall_clock_secs` | `SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS` | whole seconds (> 0) |
| `max_query_value_bytes` | `SLUGAUDIT_MAX_QUERY_VALUE_BYTES` | bytes (> 0) |
| `max_structure_query_bytes` | `SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES` | UTF-8 bytes (> 0) |
| `max_structure_matches` | `SLUGAUDIT_MAX_STRUCTURE_MATCHES` | match count (> 0) |
| `max_structure_execution_time_secs` | `SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS` | whole seconds (> 0) |
| `max_sync_wall_clock_secs` | `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` | whole seconds (> 0) |
| `max_result_store_bytes` | `SLUGAUDIT_MAX_RESULT_STORE_BYTES` | bytes (> 0) |
| `result_active_grace_secs` | `SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS` | whole seconds (> 0) |

Validation rules for `config.json` and `SLUGAUDIT_*`:
- **Unknown keys fail closed** (`#[serde(deny_unknown_fields)]`), including legacy keys like `mechanics` (mechanics are session-only via the `mechanic` tool).
- **`0` fails closed** in both `config.json` (`ConfigError::Zero`) and `SLUGAUDIT_*` (`EnvConfigError::Zero`) — set a positive integer or omit the key.
- Startup validates both `config.json` and `SLUGAUDIT_*` before serving (`validate_startup_config`), and `config.json` (up to 1 MiB, regular file) is re-read when resolving project limits so edits apply on the next call without restarting.

## Logging and path environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG` | Tracing filter syntax (for example `info` or `slugaudit_mcp_rust=debug`). Default: `warn,slugaudit_mcp_rust=info,rmcp=error`. Invalid syntax stops startup. |
| `SLUGAUDIT_LOG_FORMAT` | `text` (default) or `json` (case-insensitive). Any other value stops startup. Logs write to `stderr` only (`stdout` is reserved for JSON-RPC). |
| `SLUGTHUG_HOME` | Base directory for the installed binary and `config.json` (`$SLUGTHUG_HOME/slugaudit/`). Must be an absolute path (relative values are ignored and fall back to `~/.slugthug`). Default: `~/.slugthug`. |
| `SLUGTHUG_CONFIG_HOME` | Home directory used by `connect` / `disconnect` when locating client config files. Default: `$HOME`. |

## Per-call parameters

`page_bytes`, `limit`, and `pattern` on individual tool requests adjust that single call rather than changing global or session configuration. See [tools](tools.md).
