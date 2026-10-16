# Operator troubleshooting

This page describes failure modes an operator can observe, the code that
produces each signal, and the remedies that exist. SlugAudit reports evidence
and states facts; it assigns no severity and renders no verdict. The operator
decides what matters.

Signals come from two places: the `health` MCP tool and the error text of a
failing tool call. `health` is strictly read-only — it never syncs, never
registers a watch, and never writes, so a health read cannot perturb the state
it reports (`src/tools/health.rs:265-275`).

## 1. Every call is slow: a full publish on every call

`ensure_current` chooses its work from the watcher's health
(`src/sync/manager_orchestration.rs:19-25`):

| Watcher state | Work done |
|---|---|
| `NeedsVerification` / `Desynced` | full publish (walk + hash the tree, re-parse changed files) |
| `Healthy` with unreconciled events | incremental reconcile (only the dirty paths) |
| `Healthy` with no events | returns the current revision, no work |
| `Unavailable` | full publish |

So a call that walks the whole tree when nothing or little changed means the
watcher is not being trusted, or does not exist.

**Signal.** The `health` response's `server.consecutive_full_publishes` field. It
increments on every full publish and resets to 0 on a successful incremental
reconcile (`src/sync/manager.rs:140-148`;
`src/tools/health.rs:163`). **A value that stays at 1 or more across calls,
instead of returning to 0, means every call is paying the full-publish cost.**
`watcher.health` and `phase` in the same response tell you which state caused
it (`src/tools/health.rs:82`, `:459-467`): `desynced` maps to phase
`restoring`, `unavailable` to phase `unavailable`.

### Inotify watch exhaustion (Linux)

A large tree can exceed the kernel's per-user watch limit. The `notify`
watcher reports this as an error; SlugAudit's error callback marks every
project `Desynced` so the next call does a full verification
(`src/watch/manager.rs:80-101`). `Desynced` is documented as "watcher detected
an integrity problem (queue overflow, watch removed, etc.)"
(`src/watch/types.rs:29-31`). Because a full publish only resets the counter
when a reconcile later succeeds incrementally, `consecutive_full_publishes`
climbs instead of resetting.

**Check:**

```bash
sysctl fs.inotify.max_user_watches
```

**Remedies** (both exist):

- Raise the kernel limit, e.g. `sysctl fs.inotify.max_user_watches=524288`,
  persisted under `/etc/sysctl.d/`.
- Reduce the watch scope with `.gitignore` / `.ignore` rules. The watcher
  computes its scope from those files, drops events for ignored paths, and
  re-reads the rules when a `.gitignore` or `.ignore` file changes
  (`src/watch/manager.rs:9-12`, `:52-56`, `:410-416`).

## 2. Watcher unavailable

**Signal.** `health.watcher.health` is `unavailable` and `health.phase` is
`unavailable` (`src/watch/types.rs:32-34`, `src/tools/health.rs:459`).

**Cause.** The platform does not support filesystem watching, or a `notify`
watcher could not be created for the process. SlugAudit logs
`filesystem watcher unavailable; SlugAudit will use full verification`
(`src/watch/manager.rs:108-110`). With `health.watcher.health = unavailable`,
`ensure_current` takes the full-publish branch on every call
(`src/sync/manager_health.rs:160-172`).

**Consequence.** Correctness is unaffected — each call re-verifies the tree
from disk rather than trusting a dirty set — but every call pays the full
walk (`src/sync/manager_orchestration.rs:23-24`).

**Remedies.** Run on a platform/filesystem where the watcher can be created,
reduce the tree the sync must cover, or raise the sync budget if a legitimate
full publish is exceeding it (§3). Nothing in SlugAudit re-enables a watcher
at runtime; watcher creation is attempted once when the sync manager is built
(`SourceSyncManager::with_watcher`, `src/sync/manager.rs:92-99`, used by the
server at `src/server.rs:123-125`).

## 3. Sync budget exhaustion

Every sync operation runs under one wall-clock budget, `max_sync_wall_clock`.
It covers a full publish including all CAS retries, or one barrier-sync
reconcile pass including all iterations, and is checked cooperatively inside
the hot loops (per discovered file, per dirty path, per barrier iteration)
(`src/model/limits.rs:78-84`). Every stage shares one deadline: discovery
(`src/sync/discovery.rs:102-128`), sampling (`src/sync/sample_batch.rs:31-35`),
diffing (`src/sync/publish_diff.rs:31-35`), revalidation
(`src/sync/revalidate.rs:36-40`), the barrier loop
(`src/sync/reconcile/barrier.rs:36-45`), and the stat sweep
(`src/sync/sweep.rs:64-99`).

**Values.** The budget is set by profile, then overridable per process
(`src/model/profile.rs`; `src/model/limits.rs:111-152`):

| Profile | `max_sync_wall_clock` | Source |
|---|---|---|
| `lean` | 120 s | `src/model/profile.rs:47` |
| `audit` | 1200 s | `src/model/profile.rs:65` |
| `adaptive` (default) | 1200 s | `src/model/profile.rs:87`; default selected at `src/model/limits.rs:175-177` |

**Knobs** (both exist):

- `SLUGAUDIT_PROFILE=lean|audit|adaptive` selects a profile
  (`src/model/profile.rs:24-29`, read at `src/model/limits.rs:112-119`).
- `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS=<seconds>` overrides the seconds value
  directly, regardless of profile (`src/model/limits.rs:150-152`).

These are environment variables read once per process
(`ResourceLimits::from_env` is computed once behind `process_limits()`,
`src/model/limits.rs:88-98`). Because the MCP server is launched by the
client, the variable must be present in the environment the client uses to
spawn `slugaudit serve`, and the server must be restarted for a change to take
effect.

A per-project profile can also be set without an environment variable, but it
can only choose a profile — it cannot set the seconds value. The project
config file `.planning/slugaudit/config.json` accepts `{"profile": "audit"}`
(`src/project/config.rs:33-49`, `:65-72`; applied at
`src/sync/publish_cas.rs:54-56`), and
`project_control {"action": "on", "profile": "audit"}` writes that file
(`src/tools/project_control.rs:104-113`).

**Signal.** A tool call fails with an error whose text contains `exceeded its
wall-clock time budget`. The exact wording identifies the stage:

- `discovery exceeded its wall-clock time budget after <N> ms`
  (`src/sync/discovery.rs:51-52`)
- `publish exceeded its wall-clock time budget after <N> ms while processing
  <path>` (`src/sync/publish.rs:59-60`, raised from
  `src/sync/publish_diff.rs:31-35`, `src/sync/revalidate.rs:36-40`,
  `src/sync/sample_batch.rs:119-124`)
- `reconcile exceeded its wall-clock time budget after <N> ms while processing
  <path>` (`src/sync/reconcile/error.rs:39-40`,
  `src/sync/reconcile/pipeline.rs:90-94`)
- `stat sweep exceeded its wall-clock time budget after <N> ms`
  (`src/sync/sweep.rs:36-38`)

**What to raise.** `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`, or switch
`SLUGAUDIT_PROFILE` to `audit` or `adaptive` (both 1200 s). The work is not
interruptible mid-file: Tree-sitter parsing of a single file cannot be
cancelled once started, so one pathologically large or deeply nested file can
consume a large share of the budget in one unit of work. A larger budget is
the lever for a legitimately large first import; excluding the file
(`.gitignore` / `.ignore`) is the lever for one pathological file.

## 4. Oversized repo: the sync-budget failure loop

If a full publish cannot finish inside the budget, it fails closed rather than
publishing a partial view — discovery aborts with `TimeBudgetExceeded`
(`src/sync/discovery.rs:106-111`), and the stat sweep does the same because "a
partial sweep that silently skipped the rest of the tree would serve exactly
the staleness the sweep exists to prevent" (`src/sync/sweep.rs:64-67`). When
this happens on the first import, no revision is ever published, so every
subsequent call repeats the same full publish and the same failure.

**Signal.** `health.phase` is `importing` (documented as "no current revision
— the first sync has never succeeded", `src/tools/health.rs:169`) and
`health.database.revision_id` is null (`src/tools/health.rs:105-108`), on a repo whose
size does not justify it — or on a repo that is genuinely large.

**Remedies**, in order of least disruption:

1. Raise the budget: `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`, or profile `audit`
   / `adaptive` (§3).
2. Shrink what the sync must cover with `.gitignore` / `.ignore` rules; the
   discovery walk and the watch scope both honour them
   (`src/watch/manager.rs:9-12`).
3. Run the project on a local filesystem (§5).

## 5. The database is on a network filesystem

SQLite's locking is unreliable on NFS/CIFS/SMB mounts, so SlugAudit refuses to
open a database it detects on one (`src/store/netfs.rs:4-19`).

**Signal.** An open failure with the text `could not verify whether the
database is on a network filesystem (inspection failed: …)`. This is
`StoreError::NetworkFilesystemCheck` (`src/store/connection.rs:33-41`), which
is deliberately not classified as corruption (`src/store/connection.rs:44-54`)
— no discard or rebuild is attempted.

**Remedies.** Move the project to a local filesystem, or make the platform's
filesystem-inspection facility available: `/proc/self/mountinfo` on Linux,
`stat -f` on macOS (`src/store/netfs.rs:67-78`, `:142-156`).

## 6. The database is disposable

`.planning/slugaudit/project.db` holds only derived data — an index rebuilt
from source (`src/project/database_path.rs:5-14`). Deleting the file is safe:
the next `ensure_current` recreates it and, finding no project metadata or
current revision, rebuilds it in one full publish
(`src/sync/manager_orchestration.rs:61-99`, `:178-195`; `publish_from_scratch`,
`src/sync/manager_meta.rs:258-277`). The same rebuild happens automatically
when the stored metadata is corrupt or names a different project root
(`src/sync/manager_orchestration.rs:41-60`, `:102-136`). The cost of a rebuild
is one full publish, bounded by the same sync budget as any other full publish
(§3).

Deleting the whole `.planning/slugaudit/` **directory**, not just the
database, turns the project off: that directory is the activation marker, and
without it every tool call fails with `SlugAudit is not enabled for this
project or any parent directory`
(`src/project/activation.rs:18-19`, `:103-106`) until the agent calls
`project_control {"action": "on"}`. `project_control {"action": "off"}` is
the supported way to remove it (`src/project/activation.rs:126-142`,
`src/tools/project_control.rs:132-135`); `{"action": "on"}` recreates it and
runs the initial import (`src/project/activation.rs:120-123`).

## 7. Reverting an update or install

Both installers retain the binary they displace as `slugaudit.prev` beside the
one they publish (`~/.slugthug/slugaudit/slugaudit.prev` for the default
install): `slugaudit update` before its replacement rename
(`src/update.rs::apply_update`), `slugaudit install` before its own
(`src/install.rs::run_install`), and `install.sh` before either of its install
paths. The copy keeps the original's permission bits, so reverting is one copy:

```sh
cp ~/.slugthug/slugaudit/slugaudit.prev ~/.slugthug/slugaudit/slugaudit
```

Agent configs point at the stable path, so nothing else needs to change. The
retained copy is overwritten by the next install or update and is never read by
the running program — it exists only for this. If the copy cannot be written, the
install or update **fails** rather than proceeding: a revert path that silently
does not exist is worse than a refused update.

A revert is only needed when the new build misbehaves; the project databases are
untouched by either operation (§6), so no index work is involved.

For a release you no longer have locally, `slugaudit --version` reports the
running version, and the release page for that tag has the binary and its
`SHA256SUMS`.
