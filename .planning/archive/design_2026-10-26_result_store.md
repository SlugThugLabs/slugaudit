# Design record: immutable paged results (the result store)

Architect's design for Phase 9 of `.planning/PLAN.md`, approved by the owner on
2026-10-26. Evidence cites the tree at `2ad7bc3` on `fix/deep-dive-2026-10-25`.
This is the record a builder reads before touching paging; the plan carries the
work items and their Definitions of Done.

## Problem

Pages are recomputed from the live database on every call with `LIMIT`/`OFFSET`
(`src/tools/leads_query.rs:72`, `src/tools/query_exec.rs:214`,
`src/tools/finding_read.rs:257`; `structure` skips matches in Rust over a scan
window, `src/tools/structure_exec.rs:47,187`). Every call syncs first, so a file
change between page one and page two publishes a new revision and renumbers the
rows. The optional `expected_revision` pin (`src/tools/context.rs:159-173`)
detects this and refuses with `stale_revision`; without the pin, page two is
served from the new ordering and rows are silently skipped or repeated. The
index holds one state (superseded rows are overwritten or deleted at publish),
so page two of the old revision cannot be computed after the change.

## Contract (owner, 2026-10-26)

1. New requests always sync and describe the current tree.
2. A paged result is one logical answer; every page describes the state the
   first page was taken from, with its original ordering, rows and totals.
3. Edits made while reading never alter the result being read.
4. No silent inconsistency: nothing skipped, repeated or mixed across
   revisions; resource exhaustion and expiry are explicit errors.
5. The agent manages no revisions, no restarts, no snapshots. It copies one
   cursor string.
6. Existing strengths (sync, counts, incremental indexing, disposable
   database, publish/CAS guarantees) are untouched.
7. The grace period never invalidates a report; it only governs eligibility for
   reclaim under pressure, and continuing a result renews its protection.

## Verified facts the design rests on

- Connections are opened per call and dropped at the end; the read path is a
  DEFERRED transaction whose first statement verifies the current revision
  (`src/tools/context.rs:20-56,122-142`). Nothing holds a connection across
  calls. The read-write path requires WAL (`src/store/connection_guards.rs:293-321`).
- Probe (rusqlite 0.37, bundled SQLite 3.50.2): a DEFERRED read transaction
  held open sees the pre-commit rows after another connection commits
  (snapshot isolation), and `wal_checkpoint(TRUNCATE)` reports busy with frames
  left until that reader closes. A `READ_ONLY` connection can create TEMP
  tables while writes to the main database are refused.
- Four paths unlink `project.db` and sidecars at runtime; only
  `project_control off` guards against readers
  (`src/sync/manager_orchestration.rs:225`, `src/tools/project_control.rs:300`,
  `src/store/discard.rs:51-85`, `src/project/activation_lock.rs:22-40`).
- `leads` orders by `f.path, e.start_line` with no unique tie-breaker
  (`src/tools/leads_mechanics.rs:38-47`).
- `query` computes `total_rows` by a separate `count(*)` under its own step
  budget and reports `total_rows_unavailable` when it runs out
  (`src/tools/query_exec.rs:182-190`).
- The mechanic scan cache already materialises rows in memory per revision,
  32 MiB FIFO, on `SourceSyncManager` (`src/project/mechanic_scan_cache.rs`,
  `src/sync/manager.rs:118`). One server and one manager per process.
- Bundled build: `SQLITE_TEMP_STORE` not set (default 1), `cache_size` default
  -2000 (≈2 MB), page size 4096, `SQLITE_THREADSAFE=1`.
- Probe: `Connection::open("")` creates a private file under `/var/tmp` that is
  already unlinked on first write; 300,000 rows × 1 KiB (≈410 MB on disk) left
  RSS at 5.7 MB; `abort()` left no file; every storage class round-tripped
  through untyped columns identically (NULL, i64 extremes, REAL, +Inf, TEXT
  with embedded NUL and 4-byte UTF-8, BLOB, numeric-looking TEXT) **except
  REAL NaN, which SQLite stores as NULL**. A missing or read-only
  `SQLITE_TMPDIR` is silently skipped in favour of the next candidate directory.

## Options and why they were eliminated

- **Keyset paging alone**: prevents skip/repeat of rows already shown but does
  not freeze the result (rows inserted after the cursor appear, deleted ones
  vanish, `total` drifts). Fails contract 2 and 3. Used only as the ordinal
  walk inside a stored result.
- **Held read transaction per result**: works mechanically (probe) but needs
  one connection per open result, blocks WAL checkpoints for as long as any
  sweep is open, is exposed to the three unguarded unlink paths, and does not
  fix ordering for arbitrary SQL without a total `ORDER BY`. Eliminated.
- **In-memory Vec materialisation**: correct but memory-bound; needs a low
  hard limit that bites legitimate large reads. Superseded by the next option.
- **SQLite's own temp database (`open("")`)**: bounded memory and self-cleaning
  (probe), but the storage location is chosen by silent fallback and the
  all-candidates-unusable case could not be made to fail. Rejected for the
  location policy, not for the mechanism.
- **Chosen: one SQLite file per result in a directory we own.** Same bounded
  memory and cleanup properties, explicit location and explicit errors.

## Architecture

**Location.** `<project>/.planning/slugaudit/results/`, inside the runtime
directory that discovery, the watcher and reconcile already exclude
(`src/ignore_rules`), on the filesystem the network-mount guard already vetted.
Phase 9 adds a test proving exclusion for this subdirectory specifically.

**One file and one connection per result.** Name
`<session-uuid>-<result-uuid>.db`. Opened read-write, then immediately
unlinked by us (Unix semantics: the open descriptor keeps the inode). Pragmas:
`journal_mode=OFF`, `synchronous=OFF`, `locking_mode=EXCLUSIVE`, default page
cache. A crash leaves at most a file created in the instant before unlink;
the startup sweep removes any `results/*.db` whose owner lock it can acquire.
Each live process holds `results/<session-uuid>.lock` with an advisory lock so
two processes on one project never remove each other's files.

**Platform policy.** Supported runtime targets are Linux and macOS; both keep an
unlinked open file readable. On any other platform the store fails closed at
open with `result_store_unavailable` (the same policy `src/store/netfs.rs`
applies to unknown mounts), and the startup sweep is the only cleanup path.
This is stated in the error's remedy and in `docs/TROUBLESHOOTING.md`.

**Schema per result file.**

```sql
CREATE TABLE meta (
  key TEXT PRIMARY KEY, value            -- tool, project, revision_id, state,
);                                       -- row_count, bytes, columns (JSON),
                                         -- completeness (JSON), created_unix
CREATE TABLE rows (
  ord INTEGER PRIMARY KEY,               -- source order, assigned at materialisation
  nonfinite TEXT,                        -- JSON [[col_index, "NaN"|"Infinity"|"-Infinity"], ...] or NULL
  c0, c1, c2, ...                        -- one untyped column per result column
);
```

Untyped columns keep each cell's storage class exactly. `nonfinite` is written
at materialisation by the same detection the query path uses today
(`src/tools/query_value.rs:55-78`); at page time those cells are emitted as
`null` in `rows` and listed in `non_finite_cells`, exactly as today.

**Materialisation is atomic.** Inside the tool's existing verified read
transaction (one revision, snapshot-isolated), the tool streams its rows and
computes its counts/totals. Rows are written in batches of 1,000 inside one
result transaction; the `meta` rows with `state='complete'`, the exact row
count, revision, columns and completeness are written last; the registry entry
that makes the cursor resolvable is inserted only after commit. Any failure
(deadline, VM-step budget, store budget, I/O) rolls back, closes, removes the
file, and returns a typed error with no cursor. Page reads require
`state='complete'`. Result ids are random UUIDs.

**Cursor.** Opaque token `v1.<process-nonce>.<result-uuid>.<next-ord>`
(base64url). The position lives in the token, not on the server, so a repeated
call with the same token returns the same page; pages are disjoint by
construction. Each page returns `page.cursor` (next) or `cursor: null` with
`complete: true`. Tokens from another process → `unknown_cursor` (remedy: the
server restarted; ask again). Malformed → `unknown_cursor`. Reclaimed →
`result_expired` (remedy: ask again).

**Continuation does not sync.** A call carrying `cursor` reads the stored
result and nothing else: no sync, no revision check, no `path` needed. It is,
with `health`, one of two calls that do not bring the project current. A call
without `cursor` is a new request and syncs as every tool does today.
`offset` and `expected_revision` are removed from every paging tool.

**Retention and admission.** Registry classes: *active* (unfinished, a page
served within `result_active_grace_secs`, default 900), *idle* (unfinished,
older), *delivered* (last page served). Continuing a result sets its last
access, so an idle result that is continued becomes active again; the grace
period never deletes anything by itself. A new materialisation that fits
`max_result_store_bytes` (default 4 × `max_query_response_bytes`: 1 GiB on
`audit`/`adaptive`, 128 MiB on `lean`) is admitted. One that does not fit
reclaims delivered results oldest-first, then idle oldest-first, never active;
if it still does not fit it is refused with `result_store_full` naming the
active count and the remedy (`LIMIT`, narrow, or finish open sweeps). A single
result larger than the whole budget fails with `result_too_large`. Delivered
results are retained (not dropped on the last page) so a final-page retry
works; they are the first reclaimed.

**Concurrency.** No shared connection. Materialising result A never blocks
paging result B. The registry map is held for map operations only. Each
result's connection sits behind its own mutex. The existing semaphore of
eight blocking operations bounds concurrent materialisations. Separate
processes share only the directory and the owner-lock protocol.

**Completeness.** `meta.completeness` is `complete` or `partial{limited_by,
resume_after_file, files_scanned, files_total}`. `query` is all-or-nothing:
the whole statement is materialised within the call's budgets or the call
fails; `total` is therefore always exact and the separate `count(*)`,
`total_rows_unavailable` and its budget are removed. `leads` built-in
mechanics are full selects, always complete. `structure` and custom-mechanic
scans run under a file window and the call deadline and may stop early; the
stored result is `partial`, every page says `complete_scan: false` with
`limited_by` and `resume_after_file`, and resuming is a new request that syncs.

**Fidelity.** Rows for `leads`, `structure`, `finding_read` and `mechanic` map to
fixed columns through one `Materialize` trait (`columns()`, `to_values()`,
`from_values()`); `query` rows use the statement's column list. At page time
the existing `row_to_json`, oversized-first-row clipping (`clipped_cells`,
`continue_with`) and `non_finite_cells` run unchanged on the stored cells.
`max_query_value_bytes` stays enforced at the source read. `leads` gains
`e.id` as the final `ORDER BY` key so ordering is deterministic within a
revision. Golden fixtures captured from `2ad7bc3` before any change are the
oracle: the new responses must equal them field for field apart from the
paging fields that change by design (`cursor` replaces `next_offset`/`offset`).

**Observability.** Store stats (results by class, bytes, budget, reclaims by
class, refusals, unknown/expired cursor counts) in `health` detail. One log
line per materialisation and per reclaim with result id, tool, revision,
bytes, and class; never rows.

## Decisions open to the owner

- Continuations do not sync (a new call reports the moved tree).
- Delivered results are retained until reclaimed, not dropped at once.
- `query` is all-or-nothing; a partial arbitrary query has no honest resume.
- Budget multiplier 4 and grace 900 s are reasoned starting points, to be
  confirmed on the 68k-line project under the `audit` profile before release.
- Folding the mechanic scan cache into the result store is a follow-up, not
  part of Phase 9.
