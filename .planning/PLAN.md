# SlugAudit plan

Last updated: 2026-10-31. The owner approves every step before it is
committed. AGENTS.md is the product definition; this file is the work order.

## Goal

Make the tool accurate and fast for Rust: every answer matches the code on
disk at the moment of the call, safely, on repos up to Photoshop size (~40k
files) on a normal laptop. Nothing else gets built until that is true.

## Done (uncommitted, gate green)

| Step | What |
|---|---|
| C1 | Deleted the docs-drift, governance and version-consistency gates. The `PACK_VERSION` ↔ `Cargo.toml` pin check is kept as a unit test in `src/parse/mod.rs`. |
| C2 + C3 | Every call re-hashes every file and repairs stored text whose hash no longer matches (`src/sync/verify_stored.rs`). Deleted the watcher, sweep, reconcile, barrier and health-state layers, and the `notify` dependency. |
| C4 | Deleted the DB guards: SQL authorizer, permission checks, network-filesystem refusal, discard and activation locks. Kept bounded opens, symlink refusal, and the query step and time budgets. |
| Notices | Rust is the only tested language. `report`, `structure` and `leads` carry `notices` for other languages; the server instructions say so. |
| Repo text | `README.md`, `AGENTS.md`, and the `lhm.plugin.json` listing text corrected. |

## In progress

- `docs/`: end-user documentation.
- Two comment-alignment passes over all source files: the second follows the
  first and catches misses.
- After both: full gate, then the owner reviews the commits.

## Next, in order

1. **Commit** (owner OK needed), three commits on main, no push:
   C1 with the gate deletions; C2–C4; notices + docs + README/AGENTS/listing
   + comment alignment.
2. **Decide the activation gate** (owner). Option A, Fable's pick: no on/off.
   Root is the nearest `.git`; DB in the user cache dir. Option B: keep
   on/off as create/remove only.
3. **Rust invariant tests through the real tools** (`query`, `structure`,
   `leads`, `report`, `finding_read`): edit, add, delete, rename, file
   becomes non-UTF-8, file replaced by a directory, `.gitignore` change,
   DB deleted mid-session, FIFO in the tree (must not hang). These are the
   safety net for everything after.
4. **Correctness fixes:**
   - Corruption hit mid-sync must delete and rebuild the DB, not fail every
     call after (today only open-time corruption rebuilds).
   - Non-UTF-8 files keep their real content hash instead of a size+mtime
     identity (they are already read in full). Delete the unused
     `hash_io::hash_file`.
5. **Speed and memory fixes:**
   - Two-pass sync: stream-hash every file, then read only changed files.
     Today every call holds the whole repo's text in RAM. Add a test that
     pins peak memory on a no-change call.
   - Hash on about 4 threads (check what the code does today).
   - Per-call time limit cut from 120 s / 1200 s to tens of seconds.
6. **C5, slimmed:** delete the mechanic result cache; reduce config to the
   few knobs a user needs. Delete `check_performance`, the criterion benches
   and the `criterion` dependency.
7. **Soak test timing:** time a second call on an unchanged 10k-file tree,
   and a call after one edit, with a hard limit. This replaces
   `check_performance`.
8. **Cursor freshness:** a cursor call re-checks the files, and the page says
   which revision it came from and what disk is at now. Don't fail the page:
   agents edit while they read.
9. **Test cleanup:** delete tests for removed security and old design, and
   low-value wording tests (list in the test audit).
10. **Release:** pick the version, tag it, update every listing the same day.
    The last release is v1.0.10; `Cargo.toml` says 1.1.0, which was never
    released.

## Found while writing docs (fold into the steps above)

- Deleting `project.db` turns the project off instead of rebuilding it
  (`open_or_rebuild` → `NotActive`). Settle it with step 2: the DB is
  scratch, so a missing DB should rebuild.
- Leftover permission tightening and locks: 0700 activation dir
  (`src/project/activation.rs:174`), 0700 result-store dir and owner lock
  (`src/store/result_store/`), `config.lock` (`src/project/config_store.rs`).
  Result-store lock goes with step 8, `config.lock` with step 6.
- Findings live in the scratch DB and are lost on a rebuild. Decide whether
  findings move out of the DB.
- `.github/workflows/release.yml` links `docs/RELEASING.md` from every
  release body; the file was deleted. `docs/about.toml` and
  `docs/third-party-notices.hbs` (release build inputs) are restored.

## Open decisions (owner)

- Activation gate: A or B (step 2).
- File-size cap: today 32 MiB. 4 MiB keeps the worst case per file around
  10 ms on a spinning disk.
- Is the per-call DB tamper check (re-hashing stored text) worth roughly
  doubling the check cost?
- `mechanic` is missing from the `lhm.plugin.json` tool list.
- Three other directories list SlugAudit: what does each one promise?

## Not doing

- No file watcher.
- No security guards against threats nobody reported.
- No derived-row digest (Fable: it can't catch the bugs that matter).
- No removing other languages or the installer/connect/update/menu surface.
- No new features until the goal above is met.

## Reference numbers (Ryzen 9 5900X, 32 GB, ZFS NVMe, warm cache)

Per file, 1 thread, stat + open + read + BLAKE3: ~11 µs at ≤4 KB, 54 µs at
64 KB, 0.5 ms at 1 MB, 20 ms at 32 MB. A 10k-file Rust repo costs ~0.2 s per
call single-threaded. Linux-kernel size (82k files) costs ~1.9 s.
