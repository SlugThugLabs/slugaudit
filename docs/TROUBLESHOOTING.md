# SlugAudit troubleshooting and operator runbook

This is the operator-facing guide for installing, connecting, and diagnosing
SlugAudit. It describes the shipped binary's behavior. Design and invariants
are in [`.planning/ARCHITECTURE.md`](../.planning/ARCHITECTURE.md); client
registration details are in [`CLIENTS.md`](CLIENTS.md); release verification
is in [`RELEASING.md`](RELEASING.md).

Start with the `health` tool. It is read-only, never triggers a sync, and
answers even when every other call fails. Its arguments are:

```json
{"path": "/absolute/path/inside/the/project", "verbose": true}
```

Omit `path` for the watcher state of the project this server has registered.
`verbose` (default `false`) adds the operator detail this runbook refers to
under `detail`; without it the answer is only `phase`, `remedy`,
`watcher.health` and `database.revision_id`/`file_count`.
A path that is not an enabled project returns `phase: "disabled"` with a
`remedy` instead of an error.

## Install and update

### Install

```bash
# Installer script: prebuilt Linux x86_64 release, SHA256-verified, with a
# cargo-install fallback and a local-checkout build when run inside the repo.
curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash

# From a source checkout
cargo install --path .

# Stable local install of the running binary (offers to edit PATH; the offer
# is skipped when stdin is not a terminal)
slugaudit install

# Interactive setup (install, connect, disconnect, generic MCP instructions,
# or run the server)
slugaudit menu
```

The installer and `slugaudit install` place the binary at
`~/.slugthug/slugaudit/slugaudit`, the path `connect` registers when it is
present. Both retain the binary being replaced as `slugaudit.prev` beside it.
`SLUGTHUG_HOME` overrides `~/.slugthug`; `SLUGTHUG_CONFIG_HOME` overrides the
home directory used to resolve client config paths (tests use it).

If `install.sh` cannot download (non-Linux or non-x86_64), it falls back to
`cargo install --git` when cargo is available; otherwise it exits 1 stating
that a prebuilt binary exists only for Linux x86_64.

### Update

```bash
slugaudit update
```

`update` refuses before any network call unless the platform is Linux x86_64
(the only published asset). It fetches the latest GitHub release over HTTPS
using `curl`, downloads the binary and `SHA256SUMS`, compares the asset's
SHA-256 digest against the manifest entry, and only then replaces the target
with an atomic rename. On any failure the existing binary is untouched.

Successful outcomes print one of:

- `slugaudit is already up to date (vX.Y.Z).` — no download or replacement.
- `A newer release (tag vX.Y.Z) is available; fetching...` — then the download
  and verification run.
- `This build's version (…) could not be compared with the published …;
  fetching it as though it were newer.` — a dev build with no plain dotted
  version; the fetch is deliberate, but the line does not claim a comparison
  that never happened.
- `Done. Restart any running AI sessions so they launch the new binary.` —
  the verified binary was installed.

These branches are exercised end to end on the shipped platform (Linux
x86_64) by `tests/update_cli_tests.rs`, including a failed checksum leaving
the old binary in place. The unsupported-platform refusal is a compile-gated
unit test because it cannot run on the platform that has an asset.

Failure outcomes:

- `could not run curl (required to self-update)` — install `curl`, or download
  the release manually from GitHub.
- `checksum verification failed` / `SHA256SUMS has no entry for …` — nothing
  was replaced. Retry the download or verify the release by hand
  ([`RELEASING.md`](RELEASING.md)).
- `slugaudit update only fetches the linux/x86_64 release` — the machine is a
  different platform; build from source instead.

After an update, restart any running agent session so it launches the new
binary. To revert to the version an install or update displaced:

```bash
cp ~/.slugthug/slugaudit/slugaudit.prev ~/.slugthug/slugaudit/slugaudit
```

The `.prev` copy is overwritten by the next install or update and nothing else
reads it.

## Connect and disconnect

```bash
slugaudit connect            # detect installed agents, then prompt
slugaudit connect claude     # one target
slugaudit disconnect claude  # alias: slugaudit remove claude
```

`connect` either runs the client's own `mcp add` command or merges the
`slugaudit` entry into the client's user-level config file. Every target,
command, path, and verification step is in [`CLIENTS.md`](CLIENTS.md).

Common outcomes:

- `{client} cannot be connected automatically` — the entry has no established
  automated mechanism (`goose`, `crush`). The message carries a paste-ready
  remedy, the `Reference:` it came from, and the vendor `Docs:` link. Follow
  the block; nothing was changed.
- `` `{cli}` was not found on PATH `` — install the client, or run the printed
  command yourself. Entries with a file fallback (`opencode`, `bob`, `pi`)
  write the config file instead and say which route was taken.
- `slugaudit could not register with …` — the client ran and rejected the add.
  The message shows the exact command, the failure, and what to do by hand.
  It does **not** tell you to install a CLI that just ran.
- `cannot connect: … is not an object` / `does not hold a JSON object` —
  the target config cannot be merged safely. Fix or remove the offending file
  and retry; SlugAudit never rewrites a config it could not parse.
- `Nothing to disconnect.` — no file entry, client CLI, or home directory was
  available to remove. This includes an existing client config that has no
  `slugaudit` entry: the file is left byte-for-byte untouched (not rewritten
  or reformatted). `Done.` is printed only when a registration was actually
  removed.

Config-file registration is atomic and owner-only. The replacement temp file
is created `0600` before any bytes are written, so a client config holding
another server's API tokens never becomes group- or world-readable, even
under a permissive umask. A config reached through a symlink (a dotfiles
setup) is written through to its target, so the link survives; a dangling
symlink is refused rather than replaced. A non-UTF-8 `slugaudit` binary path
is refused before any config is written for file-strategy clients.

For a client that is not in the registry, `slugaudit menu` option 4 prints a
generic JSON MCP server snippet to paste into that client's configuration.

## Database: corruption, staleness, rebuild

Runtime state is one disposable database per project:

```text
<project>/.planning/slugaudit/project.db   (+ -wal and -shm sidecars)
```

The surrounding `.planning/` directory belongs to the project and is indexed
normally; only `.planning/slugaudit/` is excluded from discovery. Removing
the database loses nothing that cannot be recomputed from source. Findings
are session-scoped, so they are not preserved across a restart either.

### Automatic recovery

The next tool call after a damaged or incompatible database recovers without
operator action:

- **Corruption / not a database** — `open_or_rebuild` logs
  `database is corrupt or outdated; discarding and re-publishing from
  scratch` (the record carries the real database path and the source error) and rebuilds from source.
- **Unsupported schema version** — a database whose `PRAGMA user_version` is
  neither fresh (`0`) nor current (`CURRENT_SCHEMA_VERSION`, currently `11`) is
  treated as corruption and discarded. There is no migration path; the schema
  is discard-and-rebuild. The 7 → 8 bump added the derived
  `unrepresentable_paths` table, so an existing version-7 database rebuilds
  from source on its first tool call.
- **Stale metadata** — a different project root, or an incompatible
  contract/schema version, is logged as `project database is stale;
  discarding and rebuilding from source` and rebuilt.
- **Interrupted publish** — WAL recovery handles a process killed mid-write;
  a database that cannot be recovered takes the corruption path.

### Manual rebuild

Keep the activation marker and delete only the database:

```bash
rm -f <project>/.planning/slugaudit/project.db \
      <project>/.planning/slugaudit/project.db-wal \
      <project>/.planning/slugaudit/project.db-shm
```

Then run any state-bearing tool call (`health` does not sync); the first call
republishes the project.

To disable and re-enable the project (which also purges findings):

```text
project_control {"action": "off", "path": "<project>"}
project_control {"action": "on",  "path": "<project>"}
```

A project is active only while `.planning/slugaudit/project.db` exists. `off`
removes the database, its `-wal`/`-shm`/`-journal` sidecars, the `results/`
directory and `config.lock`, and says so in its response (`removed`). It keeps
`config.json` (`config_kept: true`) and any other file, and removes
`.planning/slugaudit/` only when nothing is left in it; afterwards every tool
call on the path returns `project_not_enabled`, and `on` restores the settings
saved in `config.json`. It is the one destructive tool action
(`destructiveHint: true`). `on` creates
`.planning/slugaudit/`, appends `.planning/slugaudit/` to the project's
`.gitignore` when possible (reporting `added`, `already_ignored`, or
`not_written` with the reason), optionally writes a profile to
`config.json`, and runs the initial import. Removing `project.db` (or the whole
`.planning/slugaudit` directory) manually leaves the project disabled, so call
`project_control {"action":"on", …}` rather than expecting a normal tool call
to work.

The initial import reports `added`, `modified`, `deleted`, `unchanged`,
`skipped`, and `revision_id`. `added` includes files the import then skipped,
so `added + modified + unchanged` is the discovered file set; `modified` and
`deleted` describe rows that changed or disappeared relative to the previous
revision. `skipped` is a **count** of discovered files with no stored content
(oversize, unreadable, or a non-UTF-8 path), not a list; a non-zero value means
the import is incomplete, and `report.completeness` carries the per-reason
breakdown. A paged read is continued with the `cursor` from `page.cursor`, not with
the revision (see *Paged results and the result store*).

### Open-time refusals

These are deliberate guards, not corruption, and rebuilding does not fix
them:

| Message | Cause and fix |
|---|---|
| `refusing to open a database path that is a symlink` | The database path is, or became, a symlink. A symlinked `project.db` reads as inactive, `project_control on` refuses it with this message, and `project_control off` unlinks the link itself without following it. Move it to a real local file. The open uses `SQLITE_OPEN_NOFOLLOW` so this is enforced by SQLite, not by a race-prone pre-check. |
| `refusing to open a database on a network filesystem (NFS/CIFS/SMB)` | SQLite WAL is unreliable on network mounts. Move the project to a local filesystem and re-enable it. Detected with `fstatfs` on the database directory (filesystem magic on Linux, filesystem type name on macOS); generic FUSE mounts such as sshfs are not detected. |
| `could not verify whether the database is on a network filesystem` | The platform inspection failed. The open fails closed rather than risk corruption. |
| `SQLite did not enable WAL journal mode` | The filesystem cannot support WAL; it is unsupported for this database. Use a local filesystem. |
| `failed to set database file permissions` / a group- or other-readable database | A new database is created owner-only (`0600`); an existing one with broader access is rejected. Restore owner-only permissions or rebuild. |

A busy database waits up to 5 seconds (`BUSY_TIMEOUT`) before failing;
BUSY/LOCKED during activation is reported as a database-busy condition rather
than as a lock-permission error.

## Health phases

`health` returns `phase`, `remedy`, and two groups (`watcher` with its
`health`, `database` with `revision_id` and `file_count`); `verbose: true`
adds `detail` with the rest of both groups, `database_read_failed`, the
`server` counters, `system_memory`, `mechanic_cache` and `result_store`. Each group is present
or `null` as a unit; absence is never a placeholder inside a group.

| Phase | Meaning | What to do |
|---|---|---|
| `steady_state` | Watcher `Healthy`, no unreconciled events, a current revision exists. | Nothing. |
| `degraded` | Watcher `Partial` (the watch-scope walk hit `max_watch_entries` or `max_watch_depth`), no unreconciled events, a current revision exists. The project is served and the stat sweep keeps it fresh; part of the tree has no watcher. | Normal on a very large tree. Raise `SLUGAUDIT_MAX_WATCH_ENTRIES` / `_DEPTH` only if the sweep cost matters. |
| `restoring` | Watcher `NeedsVerification` or `Desynced`, or a trusted watcher has unreconciled events. The next call full-publishes. | Usually self-healing. If it persists, see *Full-publish loops*. |
| `importing` | No current revision: the first sync has never succeeded. | Let a call finish, or re-run `project_control on`. Check `max_sync_wall_clock`. |
| `unavailable` | The watcher could not be initialized. Every call full-publishes. | Check platform support and filesystem; the stat sweep still serves reads. |
| `no_active_project` | No path was supplied and this server has no registered project, or a supplied path has no watcher and no published revision. | Call `project_control {"action":"on", …}` or pass the project path. |
| `indexed_unwatched` | The project has a published revision, but this server has no registered watcher for it. Evidence is readable; no watcher health is implied. | Normal after a fresh server start; the next state-bearing call registers a watcher. |
| `unknown` | Either no `path` was supplied and the watcher is `Healthy` or `Partial` with no unreconciled events (there is no project root in this answer, so no revision can be checked), or a supplied project's database exists but could not be read (`detail.database_read_failed` is true) while the watcher is `Healthy`, `Partial`, or not registered. Neither case claims the first sync failed. | Supply `path` for a pathless answer. For an unreadable database, see *An unreadable database* below. |
| `disabled` | The supplied path has no `.planning/slugaudit/` marker. This is a state, not an error. | The same `remedy` as `project_not_enabled`: enable the project. |

With `verbose: true` the answer adds `detail`, typed in the code
(`src/tools/health.rs`) rather than in the tool's output schema:

| `detail` field | Meaning |
| :--- | :--- |
| `watcher` | `pending_dirty`, `pending_deleted` (unreconciled paths), `sequence`, `last_verified_sequence`; `null` exactly when `watcher` is. |
| `database` | `parser_pack_version`, `skipped_file_count` (rows discovered but never read); `null` exactly when `database` is. |
| `database_read_failed` | The database exists but could not be read; see *An unreadable database*. |
| `server` | `tool_call_count`, `tool_call_error_count`, `tool_call_total_ms`, `last_sync_unix_seconds`, `last_sync_duration_ms`, `consecutive_full_publishes`. |
| `system_memory` | `total_bytes`, `available_bytes`: what sized the adaptive import budget. |
| `mechanic_cache` | `entries`, `bytes`, `budget`, `evictions`, `skipped_too_large`, `hits`, `misses` of the registered-mechanic scan cache. |
| `result_store` | The paged-result store (see *Paged results and the result store*); `null` until the first paged result is made in this process. `active`, `idle`, `delivered` (each `results` and `bytes`), `budget_bytes`, `admitted_bytes`, `reclaimed_delivered`, `reclaimed_idle`, `refused_full`, `refused_too_large`, `unknown_cursors`, `expired_cursors`, and `sweep` (`removed_files`, `live_owners_skipped`, `errors`, `first_error`). |

`unknown` exists precisely so the answer never claims the first sync failed
(that would be `importing`) or that a revision exists (that would be
`steady_state`). A pathless answer is not an error and needs no action beyond
passing a path if database state is wanted.

### An unreadable database

When the project database exists but could not be inspected, opened, or
queried, `health` returns `database: null` with `detail.database_read_failed:
true` (ask with `verbose: true`).
The phase is `unknown` when the watcher is `Healthy`, `Partial`, or not
registered; the watcher-only phases (`unavailable`, `restoring`) still
describe the watcher and stay truthful. The failure may be permissions,
corruption, a symlink or network filesystem refused at open, a lock/timeout,
or schema drift; stderr logs the failed inspect/open/read beside the real database
path (`path=`) and the source error. The phase deliberately does not say `importing` — "the first sync has
never succeeded" is false when the revision exists but cannot be seen.

`health` is read-only and never syncs or rebuilds. The next state-bearing call
opens the database and applies the normal recovery path (corruption and schema
mismatch are discarded and rebuilt; see *Automatic recovery*). If the
condition persists, check the *Open-time refusals* above and restore local,
owner-only, non-symlinked storage.

`detail.server` counters are instrumentation and drive no phase: total tool calls,
errors, total milliseconds, last sync time/duration, and
`consecutive_full_publishes`.

## Watcher health and full-publish loops

| Watcher health | Behavior |
|---|---|
| `Healthy` | If the watcher has no unreconciled events and the last full-tree sweep started less than 30 seconds ago (`FULL_RECONCILE_INTERVAL`) with the same ignore rules, the sweep is skipped and the current revision is returned. Otherwise: stat sweep, then reconcile unreconciled events if any. |
| `Partial` | Same, but the watch scope stopped at a documented cap. The stat sweep remains the freshness guarantee for unwatched subtrees. |
| `NeedsVerification` / `Desynced` | Full publish, drain events that arrived during it, then `Healthy`/`Partial`. |
| `Unavailable` | Full publish on every call. |

`detail.server.consecutive_full_publishes` counting 1+ across repeated calls means the
watcher is not being trusted. On Linux, inotify watch exhaustion
(`fs.inotify.max_user_watches`) still makes `notify` fail if the kernel limit
is below the number of scoped directories; the manager then marks the project
`Desynced` and every call full-publishes. Raise the kernel limit, or raise
`SLUGAUDIT_MAX_WATCH_ENTRIES` / `_DEPTH` if the scope walk itself is the
limit. A failed stat sweep also fails closed: it sets `Desynced` and the call
errors rather than serving stale evidence for the unwalked remainder.

Watch registration is **non-recursive**. The scope walk enumerates every
indexable directory and each is registered individually, so the kernel watch
count stays bounded by `max_watch_entries` instead of growing with the whole
subtree (including `.git`, `target`, and other ignored trees). A directory
created after the scope was computed marks the scope dirty and is registered
on the next scope refresh (the next tool call); files created inside it before
then are caught by the per-call stat sweep. A changed watch set also forces a
full verification, so this is a latency window, not a freshness gap.

Ignore rules are **project-scoped**. Discovery, the watch-scope walk, the
sweep, and reconcile read `.gitignore`, `.ignore`, and `.git/info/exclude` at
or below the project root; ignore files in ancestor directories above the
root are not applied. A project nested inside another repository can
therefore index a file that the outer repository's rules would exclude — add
the rule to the project's own ignore files if you want it honored. One rule
set is shared by the full and incremental paths, so they cannot disagree about
what the index contains.

The watcher is a hint; the stat sweep is the guarantee, and on a `Healthy`
watcher it runs on a bound rather than on every call. A call with an empty
event queue skips the whole-tree walk, and a full sweep runs on the first call
after start, on any call after the watcher left `Healthy` (a full
verification), when the ignore rules changed, and on the first call once 30
seconds (monotonic) have passed since the last completed full sweep. A failed
sweep never counts as completed. So the freshness bound is: an answer reflects
every change the watcher reported by the time of the call, and any change it
did not report (a dropped event, or a file name that is not valid UTF-8, which
the event path may not represent) within 30 seconds of the last full sweep. A call
made within milliseconds of a write, before the operating system has delivered
the event, can also miss it until the next call or the 30 second sweep.
`Partial` and `Unavailable` watchers still sweep or publish on every call. The
documented blind spot is a same-size in-place edit in the same wall-clock
second with no watcher event.

## Logs

- Logs go to **stderr** only. stdout is the JSON-RPC transport in serve mode.
- `RUST_LOG` sets the filter; unset, the default keeps this crate at `info`
  and its dependencies at `warn`. An invalid or non-UTF-8 value stops startup
  with an error naming `RUST_LOG` (never its value); fix or unset it.
- `SLUGAUDIT_LOG_FORMAT=json` (any letter case) switches to one-line JSON
  events; unset or any other UTF-8 value (such as `text`) uses plain text with
  ANSI disabled. A non-UTF-8 value stops startup with an error naming the
  variable.
- Logging is initialized before command dispatch, so CLI warnings
  (`probe`, `install`, `update`) reach the subscriber.

Every tool call emits exactly one completion event, `tool call completed`, at
INFO on success and WARN on failure, with `outcome`, `elapsed_ms`, and
`ToolCounters` deltas. All lines for a call share a `tool_call` span carrying
`tool`, `call_id`, and `session_id`. To recover one session's call sequence:

```bash
# JSON format
grep -F '"session_id":"<uuid>"' audit.jsonl \
  | jq -r 'select(.fields.message == "tool call completed")
           | [.span.call_id, .span.tool, .fields.outcome, .fields.elapsed_ms] | @tsv'

# Text format
grep -F "session_id=<uuid>" audit.log | grep -F 'tool call completed'
```

Paths: operator-facing log lines name the real path (`path=`) and the source
error so an operator can act on them; the local operator owns the machine.
SQL text, finding titles and
descriptions, indexed source content, and environment values never enter logs.
A few one-shot diagnostics whose purpose is to name the file just operated on
(for example a leftover staged-binary path) may print a raw path. Error codes
and remedies are static strings; agent-supplied input appears only in the
human-readable message, never in `ErrorData.data`.

## Machine-readable failure codes

A failing tool call may carry `ErrorData.data` as
`{"code": …, "remedy": …}`. The set is closed and defined in
`src/error_codes.rs`:

| Code | Condition | Remedy |
|---|---|---|
| `sync_budget_exhausted` | `max_sync_wall_clock` was spent during discovery, hashing, parsing, or publishing. On a tree too big for the budget this is permanent. | Raise `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` (seconds), or run `SLUGAUDIT_PROFILE=audit` (1200 s) instead of `lean` (120 s). Retrying unchanged fails identically. |
| `project_not_enabled` | No ancestor of the path has a `.planning/slugaudit` activation marker. | Call `project_control` with `{"action":"on","path":"<project directory>"}`. |
| `stale_revision` | The revision this call pinned is no longer current (another publish won the CAS race, or a sampled file changed before the write). | Retry the call; the sync in front of it re-reads the index and pins the new revision. |
| `unknown_cursor`, `result_expired`, `result_store_full`, `result_too_large`, `result_store_unavailable`, `paging_parameter_removed` | Paging. | See *Paged results and the result store*. |
| `unknown_mechanic` | `leads` was asked for a mechanic id not in the catalogue. | Call `leads` with only `path` to list every valid id with its count, then retry with one of them. |
| `sql_too_large` | SQL passed to `query` exceeded `max_query_sql_bytes`. | Shorten the SQL, or raise `SLUGAUDIT_MAX_QUERY_SQL_BYTES` (bytes). |
| `file_not_indexed` | The named file has no indexed source content: never discovered, skipped as oversize/unreadable, or binary. | Check what is indexed with `SELECT path FROM files WHERE path LIKE …`. A skipped file has no content to read; `report.completeness` says why it was skipped. |

`health` is the deliberate exception: a path that is not an enabled project
answers `phase: "disabled"` plus the same `project_not_enabled` remedy rather
than failing.

## Paged results and the result store

`leads`, `query`, `structure`, `finding_read` and `mechanic list` page through a
**stored result**. The first call (no `cursor`) syncs, reads its rows out of one
consistent snapshot and writes them to the result store; every page, including
the first, is served from that copy. `page.cursor` names the next page; repeat
the call with only that `cursor`. A continuation does not sync and does not
check the revision, so it returns the rows, totals and `revision_id` of the
first page however the files changed since. A call without `cursor` is a new
request and describes the tree as it is now. `offset` and `expected_revision`
no longer exist.

**Location.** One SQLite file per result under
`<project>/.planning/slugaudit/results/`, named `<session-uuid>-<result-uuid>.db`.
Each file is unlinked as soon as it is created (the open handle keeps it
alive), so a live result is not visible in a directory listing. The directory
is inside the runtime directory that discovery, the watcher and reconcile
already exclude, and it holds `<session-uuid>.lock`, an advisory lock that marks
a live server.

**Cleanup.** A result disappears when the server exits, when it is reclaimed
to make room, or when the store is dropped by a project switch. At startup a
sweep removes `results/*.db` files whose owner lock it can take (a crashed or
killed server); files of a server that is still running are left alone. The
outcome is `detail.result_store.sweep` in `health`; a sweep problem is logged
and counted but does not disable paging. Cursors do not survive a server
restart. Safe to delete by hand: any file in `results/` while no server runs
for the project.

**Retention.** Unfinished results served within `result_active_grace_secs`
(default 900) are *active* and are never reclaimed. Older unfinished results
are *idle*; fully read results are *delivered* and are kept for a final-page
retry. When a new result does not fit `max_result_store_bytes` (4 times
`max_query_response_bytes` unless set itself), delivered results are reclaimed
oldest first, then idle ones. Reading a result renews it. Both limits can be
set by environment variable (`SLUGAUDIT_MAX_RESULT_STORE_BYTES`,
`SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS`) or in `config.json`
(`max_result_store_bytes`, `result_active_grace_secs`); `0` is refused with a
warning.

**Platform policy.** The store needs an unlinked file to stay readable, which
Linux and macOS provide. On any other target it fails closed at open with
`result_store_unavailable`. Only the paged tools are affected; tools that do not
page (`report`, `health`, `finding`, `project_control`) keep working.

| Code | Cause | Remedy |
|---|---|---|
| `unknown_cursor` | The cursor is malformed, was issued by another tool, belongs to a server that has since restarted, or points past the end of its result. | Copy the cursor exactly as returned, or repeat the request without one; a cursor does not survive a server restart. |
| `result_expired` | The cursor is valid but its result was reclaimed to make room. | Repeat the request without a cursor. |
| `result_store_full` | The new result does not fit the budget and everything that could be reclaimed has been; the rest are active reads. | Finish or abandon open paged reads, or narrow the request (`LIMIT`, filters). |
| `result_too_large` | The one result is larger than the whole store budget. | Narrow the request: add `LIMIT` or filters; the whole result must fit the store budget. |
| `result_store_unavailable` | The store cannot be used: an unsupported platform, or the disk is full or unwritable, or the `results/` directory cannot be created or locked. | See this section; free disk space or fix the directory permissions. Other tools keep working. |
| `paging_parameter_removed` | The request carried `offset` or `expected_revision`. | Pass the cursor from the previous page's `page.cursor`, or omit it for a new request. |

## Sync budget, profiles, and limits

`max_sync_wall_clock` bounds one sync operation from discovery through publish.
The default profile is `adaptive`; `SLUGAUDIT_PROFILE=lean|audit|adaptive`
selects a preset. The wall-clock budgets are:

| Profile | `max_sync_wall_clock` | `max_file_bytes` | `max_total_import_bytes` |
|---|---:|---:|---:|
| `lean` | 120 s | 32 MiB | 2 GiB |
| `audit` | 1200 s | 256 MiB | 32 GiB |
| `adaptive` | 1200 s | host RAM / 10, at least 256 MiB and at most `SQLITE_MAX_LENGTH` | host available RAM / 2, minimum 4 GiB |

Every field has a `SLUGAUDIT_<FIELD>` override, for example
`SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`, `SLUGAUDIT_MAX_FILE_BYTES`,
`SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES`. The full list is the field documentation
in `src/model/limits.rs`. The result store that backs paged results is bounded by
`SLUGAUDIT_MAX_RESULT_STORE_BYTES` (4 x the profile's response ceiling by default: 1 GiB on
`audit`, 128 MiB on `lean`) and protects an unfinished result for
`SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS` (900) after its last page. Only explicitly set variables override a default;
an unset or unparseable value leaves the default in place (a set-but-invalid
override is reported in a warning).

Two ordering rules matter:

- Process limits are resolved once (`OnceLock`). Changing an environment
  variable after the first tool call has no effect; restart the server.
- A project's `.planning/slugaudit/config.json` `profile` **replaces** the
  whole env-derived limit struct. The watch-scope caps are the exception: the
  watcher is registered before the project config is loaded, so
  `max_watch_entries` / `max_watch_depth` come from the process environment
  only.

`max_total_import_bytes` is a **work budget**, not a memory bound. Peak memory
also includes in-flight samples, evidence payloads, and parse transients; a
tree can fit the budget and still exhaust RAM. A separate memory bound is not
implemented.

One sync uses one deadline across all phases and CAS retries; a pathological
repository cannot restart the clock. Reconcile loops at most 16 barrier
iterations; exceeding that marks the watcher `Desynced` and returns
`BarrierCapExceeded` rather than draining forever.

## Query, structure, and paging limits

| Symptom | Meaning and action |
|---|---|
| `sql_too_large` | The SQL exceeds `max_query_sql_bytes` (lean 50,000; audit/adaptive 200,000 by default). Shorten it or raise the override. |
| `page.complete` / `page.cursor` / `page.limited_by` | The response was cut at `max_query_response_bytes` (or the requested `page_bytes`, capped by it) on a row boundary; `page.complete` is false, `page.cursor` is set and `page.limited_by` names the cap. Continue by repeating the call with only that `cursor`, or request a larger `page_bytes`. A single row larger than the page is returned whole. |
| `page.completeness` is `partial` | `structure` (or a custom mechanic scan) stopped early at `page.completeness.limited_by` (`scan_window` or `time_budget`). The stored result is complete as far as it goes; to look further make a new request with `start_after_file` set to `resume_after_file`. That request syncs. |
| `unknown_mechanic` | `leads` only accepts ids from its catalogue. List them with a counts-only call. |
| `file_not_indexed` | `structure` and `finding` need indexed source content. A skipped or binary file has none. |
| Query wall-clock or VM-step failure | `query` enforces an SQLite VM-step budget and a wall-clock budget through the progress handler; `structure` enforces a wall-clock budget through Tree-sitter's progress callback. Raise the corresponding override only if the query is legitimately that large. |

Per-value and per-file limits are clamped to the bundled SQLite's
`SQLITE_MAX_LENGTH` (1,000,000,000 bytes). A file over the per-file ceiling is
stored as a `skipped` row with `skip_reason = "oversize"` rather than failing
the publish.

## Supported platforms

- **Runtime:** Linux and macOS. Any other target fails closed when opening the
  database (`src/store/netfs.rs`) rather than silently accepting an unknown
  mount or filesystem.
- **Released binaries:** Linux x86_64 only. macOS and other platforms build
  from source (`cargo install --path .` or the installer's cargo fallback).
  `slugaudit update` refuses to fetch the Linux x86_64 asset on any other
  platform.

## Report completeness counts

`report` returns counts that say whether the rest of the response is the whole
truth. Quote `file_count` together with this block, not alone.

| Field | Meaning |
|---|---|
| `file_count` | Rows in `files`: every discovered file whose project-relative path is valid UTF-8, whether or not it was read. It does **not** mean "files with evidence". |
| `completeness.file_kind_counts` | Every `files` row by `file_kind`: `indexed`, `binary` (sampled, deliberately not parsed), `skipped` (discovered but never read). |
| `completeness.file_class_counts` | Every row by path-derived `file_class`: `authored`, `generated`, `vendored`. A heuristic from the path, not a verdict. |
| `completeness.skipped_file_count` | Rows with `file_kind = 'skipped'`: present in the project but not in the index. |
| `completeness.skipped_by_reason` | Those rows grouped by `skip_reason`: `oversize`, `unreadable`, `stat_failed` (size unknown, `byte_len` NULL), or empty when no reason was recorded. |
| `completeness.extraction_completeness_counts` | Every row by `Full`, `Partial`, `ContentOnly` (text indexed, no parser ran), `Unavailable` (no content retained). |
| `completeness.index_partial` | True when an existence row is skipped or a discovered path has no UTF-8 representation, so the index does not cover every discovered file. |
| `completeness.unrepresentable_path_count` | Discovered files whose path is not valid UTF-8, so no `files` row can represent them. The count is persisted in the derived `unrepresentable_paths` table, refreshed exactly by every full publish and every incremental stat sweep (including removals), and read as `COUNT(*)` inside the report's verified read transaction — `report` does not walk the filesystem. Included in `index_partial`. |
| `completeness.evidence_capped_file_count` | Files whose stored evidence was cut short by an evidence cap (`evidence:truncated`). Evidence counts for these are a floor, not a total. |
| `completeness.indexed_files_without_evidence` | `indexed` rows with no evidence at all: content is readable but nothing structural was extracted (no parser, or parsed to nothing). Binary and skipped rows are excluded. |
| `parser_failure_count` / `parser_failure_files_total` | Failure entries and their uncapped total. `parser_failure_files` lists at most 20; a total larger than the list length means the list was truncated. |
| `credential_pattern_file_count` | Rows matching the credential-shaped path predicate (`.env`, `*.pem`, `*.key`, `id_rsa*`, …). A path count, not a content inspection. |

If `index_partial` is true, any answer built from the index is missing at
least one discovered file, and the reasons are in the same block.

## MCP surface and snapshot

The tool surface is the contract: nine tools (`report`, `leads`, `query`,
`structure`, `finding`, `finding_read`, `project_control`, `health`,
`mechanic`), their
titles, descriptions, input and output schemas, annotations, and the
pull-based resources under `slugaudit://`. `check_mcp_surface` derives the
live `tools/list` + `resources/list` from the server and compares it with
`.planning/archive/mcp_surface.json`:

```bash
cargo run --quiet --bin check_mcp_surface --locked
# deliberate surface change only:
cargo run --bin check_mcp_surface -- --record
```

A failing check means the binary and the committed snapshot disagree. The
committed snapshot currently records 8 tools and 8 resources; it was recorded
from the integrated tree, and both gates pass. If the change is intentional,
record it in the same commit as the surface change so the diff is reviewable.
If it is not, do not re-record to make the gate green. The snapshot pins the
surface; it is not a runtime file a client reads.

## Exit codes and error reporting

- Exit codes: `0` ok, `1` runtime failure, `2` usage error (unknown command or
  flag, extra arguments). A reported failure prints `Error: <reason>` to stderr.
- In serve mode, SIGTERM or SIGINT prints one line to stderr and exits `0`;
  closing stdin before the client sends `initialize` also prints one line and
  exits `0`.
- CLI output is written fallibly. A **closed pipe** (`BrokenPipe`, for example
  `slugaudit menu | head -c 10`) exits `0` with no error log: the reader went
  away, not the command. Any other write failure (for example
  `slugaudit help >/dev/full`) is reported and exits `1`.
- In serve mode there is no process-level error for a failed tool call: the
  call returns an MCP error response, and the process exits when the host
  closes stdin (end of the session).
- Invalid arguments to a CLI command print the usage error and exit `2`;
  unknown tool/request shapes are contract errors (`invalid_params`), while
  operational failures are internal errors whose message says the failure is
  often transient.
