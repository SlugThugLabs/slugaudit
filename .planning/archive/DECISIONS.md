# Decisions

Append-only decision log. Each entry records a choice that was made
deliberately, so a later reader can see why instead of re-deriving it. **Add
new entries at the top; never rewrite or delete an existing one.** If a
decision is reversed, add a new entry that names the old one.

Format: **what** / **why** / **where** / **reversible?** Dates are the date
the decision was taken, not the date it was written down. Claims carry a
commit, a path, or a command where possible.

---

## 2026-10-19 — Wave 5 verification audit: 56 confirmed P0/P1, all fixed

- **What:** 12 independent chunk auditors read **282 of 282 files (57,449
  lines)** at `66536c0` (code at `4b765e1`; later commits are docs only), and
  a verification pass reopened every cited line and enclosing function. The
  65 author candidates (17 P0, 48 P1) collapsed to **56 confirmed P0/P1
  (12 P0, 44 P1)**: 9 duplicate rows were merged into confirmed rows, **0 were
  dropped** as false, misstated, or already covered, and **9 suspected misses**
  were recorded separately but not counted. Scope: 49 production, 2 dev-gate,
  5 test. **All 56 were fixed** in `66536c0..2a65a85` (the fix set plus the
  integration fixes `730699f` and `2a65a85`). The durable records are
  `.planning/archive/audit_2026-10-19_final_report.md` and
  `.planning/archive/audit_2026-10-19_final_ledger.md`.
- **Why:** wave 4 closed the strict-audit debt, but no fresh full audit had
  re-read the tree since those fixes. Author counts are not quotable until a
  separate pass reopens each cited line; keeping the suspected misses outside
  the confirmed count avoids inflating it while still preserving the lead. The
  confirmed ledger is what the fix wave targets, and every row it contains is
  re-checkable against the copied ledger.
- **Where:** `.planning/archive/audit_2026-10-19_final_report.md`;
  `.planning/archive/audit_2026-10-19_final_ledger.md`; `.planning/PLAN.md`
  *Phase 7*; commits `643b5fe`, `d31d063`, `9fc3684`, `144f56e`, `238ab9a`,
  `db0183e`, `030907e`, `8763818`, `78a33e4`, `979b408`, `d4b204c`,
  `8fa8234`, `65076ab`, `6d2a49e`, `84a552d`, `4f3760d`, `120ad7d`,
  `730699f`, `2a65a85`.
- **Reversible:** n/a — this records an audit result and its fix set, not a
  design choice.

## 2026-10-19 — Watch registration is non-recursive; the stat sweep covers the window

- **What:** the initial project watch and every directory the scope walk
  includes are registered with `RecursiveMode::NonRecursive`
  (`src/watch/manager/scopes.rs`, `src/watch/scope_apply.rs`). The scope walk
  already enumerates every indexable directory, so each is watched
  individually and the number of kernel watches is bounded by
  `max_watch_entries` instead of by notify's recursive per-directory walk over
  the whole subtree. A directory created after the walk marks the scope dirty
  and is registered on the next scope refresh; files created inside it before
  that are caught by the per-call stat sweep. A changed watch set still forces
  `NeedsVerification`, so the window is verified rather than assumed.
- **Why:** notify 8.2.0's recursive `add_watch` adds one inotify watch per
  directory in the subtree (inotify.rs:400-414), including gitignored trees
  the scope excludes, so `max_watch_entries` did not bound kernel watches. A
  large `node_modules`, `target`, or `.git` could exhaust
  `fs.inotify.max_user_watches`; notify returned `MaxFilesWatch`, health
  became `Unavailable`, and every call full-verified. The accepted trade is a
  short unwatched window for a newly created nested directory; the stat sweep
  is already the documented freshness guarantee, so the trade is confined to
  latency, not correctness.
- **Where:** `src/watch/manager/scopes.rs`; `src/watch/scope_apply.rs`;
  `src/watch/manager/events.rs`; commit `120ad7d`; `.planning/PLAN.md`
  *Phase 7* and *Accepted and held decisions*; `docs/TROUBLESHOOTING.md`
  *Watcher health and full-publish loops*.
- **Reversible:** yes in code, but returning to recursive registration needs a
  separate kernel-watch bound; without one, the `max_watch_entries` guarantee
  is lost again.

## 2026-10-19 — Ignore rules are project-scoped (`parents(false)`)

- **What:** `indexable_walker_with_max_depth` now sets `parents(false)`
  (`src/ignore_rules.rs`). `.gitignore`, `.ignore`, and `.git/info/exclude`
  files are read at or below the project root; ignore files in ancestor
  directories above the root are **not** applied. Discovery, the watch-scope
  walk, the sweep, reconcile, and `IgnoreRules::build` all share this rule set.
- **Why:** `standard_filters(true)` had enabled `parents(true)`, so the walker
  honored ancestor ignore files while `IgnoreRules::build` only received the
  ignore files the scope walk found under the root. In a project nested inside
  another repository, full publish could exclude a file that the watcher and
  reconcile then indexed, and a later sweep deleted it — the full and
  incremental paths disagreed about what the index contains. Project scope is
  the module's documented rule set, so aligning the walker to it is the
  smaller, verifiable fix.
- **Where:** `src/ignore_rules.rs` (`parents(false)` and its comment); commit
  `84a552d`; `docs/TROUBLESHOOTING.md` *Watcher health and full-publish
  loops*; `.planning/PLAN.md` *Phase 7*.
- **Reversible:** yes, but honoring ancestor ignore files again requires the
  incremental matcher to read them too and the shared contract to say so.

## 2026-10-19 — `ImportReport.skipped` is a count, not a list

- **What:** `project_control` with `action: "on"` now returns an `import`
  report with `added`, `modified`, `deleted`, `unchanged`, `skipped`, and
  `revision_id`. `added` includes files the import then skipped (so
  `added + modified + unchanged` is the current discovered file set);
  `skipped` is a count, not an unbounded path list, and the per-reason
  breakdown remains in `report.completeness`. The field docs are part of the
  recorded MCP output schema.
- **Why:** an unbounded list would be cut by the response byte budget, and a
  row cap is not the unit this project uses — bytes are. A count is quotable
  as-is, and the call that wants detail already has `report`. Reporting
  `modified` and `deleted` closes the re-enable gap where an import that
  changed or removed rows described only additions. No response silently
  truncates.
- **Where:** `src/tools/project_control.rs`;
  `src/tools/project_control_types.rs`; commit `4f3760d`;
  `.planning/archive/mcp_surface.json`; `.planning/PLAN.md` *Phase 7*.
- **Reversible:** adding fields is compatible; replacing the count with a list
  needs a byte-budget design and a new schema record.

## 2026-10-19 — Wave 5 residual P2s and accepted connection guards

- **What:** four P2 follow-ups are recorded rather than fixed:
  1. `connect_cli::add_args` still uses `binary.display().to_string()`, so a
     non-UTF-8 binary path is lossy in the argv for CLI-strategy clients. The
     file-strategy config writer now refuses the same path with
     `ConnectError::BinaryPath` before any write; making argv construction
     fallible is the follow-up.
  2. `connect::write_atomic` creates the temp owner-only (`0600`), writes
     through a symlinked config to its target, and removes the temp on a
     failed write/rename, but does not `fsync` before the rename.
  3. The ignore-error mapping in `src/ignore_rules/build.rs` duplicates the
     watch helper; deduplicating it keeps both log contracts from drifting.
  4. Test-only R1/R2/R4/R10 P2s and dev-gate P2 notes were counted, not
     enumerated, in the wave-5 chunk ledgers (for example a zero median
     accepted by `check_performance`, an untested unreadable-CODEOWNERS arm in
     `check_governance`, impossible dates accepted by `check_source_limits`,
     and a stale canonical-set parenthetical in `check_docs_drift`).
  Accepted, not follow-ups: the connection `Configure` arms for `busy_timeout`
  and function registration remain unreachable without an invalid handle or
  OOM, and the one-shot security rejections in `connection_guards.rs` keep raw
  paths under the 2026-10-16 one-shot diagnostic carve-out. The 9 suspected
  misses from the audit are recorded in the final ledger but not counted;
  `ImportReport`'s deleted/skipped gap among them was closed by the
  `ImportReport` change.
- **Why:** none of these is a confirmed P0/P1 defect, but a finding held only
  in a chunk ledger or a session is a finding lost. Recording the residue and
  the acceptances prevents a later wave from rediscovering it or re-opening an
  arm that was already decided.
- **Where:** `.planning/archive/audit_2026-10-19_final_report.md` (*Accepted
  decisions / residual P2s*);
  `.planning/archive/audit_2026-10-19_final_ledger.md` (*Suspected misses*);
  `src/connect_cli.rs` (`add_args`); `src/connect/config_file.rs`
  (`write_atomic`); `src/ignore_rules/build.rs`;
  `src/store/connection_guards.rs`; `.planning/PLAN.md` *Phase 7*.
- **Reversible:** a follow-up that closes one of these updates the plan and
  this record; the accepted guards need a new decision to change.

## 2026-10-19 — Wave 5 final gate passed on `main` at `2a65a85`

- **What:** the integrated wave-5 tree passed the full gate:
  `cargo fmt --all -- --check`; `cargo clippy --all-targets --all-features
  --locked -- -D warnings`; **1,246 tests passed, 0 failed, 1 ignored**;
  `cargo deny check` (one unmatched license-exception warning);
  `cargo machete`; **93.56%** line coverage (12,005 / 12,831; gate 83%);
  `check_source_limits` (0 production files over 200 code lines;
  `manager_orchestration.rs` at 197); `check_no_duplicates`;
  `check_producer_coverage`; `check_governance`;
  `check_version_consistency`; `check_docs_drift`; `check_mcp_surface`; and
  `check_performance` at 0.99x–1.02x against the 20% threshold. `RUSTFLAGS='-W
  missing_docs' cargo check --lib --bins` and `cargo doc --no-deps --lib`
  report 0 warnings; `cargo crap --min 0 --path src` reports 0 production
  functions with CC > 15.
- **Why:** the wave-5 fixes touched sync, watch, tools, connect, update, and
  the gates; the integrated gate is the acceptance evidence. A later reader
  should not have to re-run the wave to learn the outcome.
- **Where:** `.planning/archive/audit_2026-10-19_final_report.md` (*Post-fix
  final validation*); `.planning/PLAN.md` *Phase 7*; commits `66536c0..2a65a85`.
- **Reversible:** n/a — this records a gate result, not a design choice.

## 2026-10-19 — Mutation-testing scope: six files, revision_support added for the moved stale-finding invalidation

- **What:** the `cargo-mutants` step now covers
  `src/sync/revision.rs`, `src/sync/revision_support.rs`,
  `src/sync/publish.rs`, `src/sync/hash.rs`, `src/sync/hash_io.rs`, and
  `src/tools/context.rs`. `revision_support.rs` was added because the wave-4
  size fix moved `invalidate_stale_findings` out of `revision.rs` into it
  without changing the logic; the other five are the wave-3 scope. This is
  not a widening of the correctness core — it is the same stale-finding
  invalidation at its new path.
- **Why:** the scope exists to mutate revision identity, atomic publication,
  CAS/change detection, and the freshness-verification path. Keeping only
  `revision.rs` after the move would leave the code that marks a changed
  file's findings `stale` unmutated, so a survivor there could serve a
  finding against a hash it no longer matches.
- **Where:** `.github/workflows/quality.yml` (the step comment and the six
  `--file` flags); `AGENTS.md` *Testing and QA → Mutation testing*;
  `src/sync/revision_support.rs`; commit `ff63113` for the size fix and
  `4b765e1` for recording the six-file scope in the workflow and AGENTS.md.
- **Reversible:** yes, but a file leaving the correctness core needs a new
  decision and must be updated in the workflow comment and AGENTS.md
  together.

## 2026-10-19 — Wave 4 landed: R8 closed, surface re-recorded, notices render validated

- **What:** wave 4 (`3dc9e42..976dc2e`, doc-link fix `27ae582`, surface
  re-record `cf1475e`) closed all 258 R8 production entries: 241 current
  `missing_docs` sites were documented (241 → 0), `cargo doc --no-deps --lib`
  reports **0 warnings**, and six external-use items were made crate-private,
  test-only, or removed from the re-export (`cli::USAGE`,
  `SlugAuditServer::with_manager`, `LineIndex::line`, `ErrorCode::as_str`,
  `WatchScopeLimit`/`WatchScope::limit`, `ErrorCode::ALL`). The MCP surface
  was re-recorded from the integrated tree (8 tools, 8 resources). The
  third-party notices were rendered through the exact workflow command with
  `cargo-about 0.9.2`: 6,558 lines / 340,298 bytes, exit 0, no warnings;
  HTML escaping fixed (`{{{name}}}`), the machine-specific `source_path`
  block removed, and `ring` 0.17.14's upstream ISC
  source-header/code-fragment limitation recorded as a tool limitation.
  Final gate on `main` at `4b765e1`: fmt, clippy, deny, machete, source
  limits, duplicate checks, producer coverage, governance, version, docs
  drift, and MCP surface pass; **1,176 tests passed, 0 failed, 1 ignored**;
  **93.27%** line coverage (11,661 / 12,503; gate 83%); performance
  0.97x–1.09x against the 20% threshold.
- **Why:** R8 was the last production half of the strict-audit debt. A public
  item kept public only for its own test is a contract the crate does not
  mean to offer, so the six external-use items were reduced rather than
  documented as supported API. The notices render is release evidence: it
  proves the checked-in template and `about.toml` produce a complete,
  path-free file under the version the workflow actually runs.
- **Where:** `.planning/archive/audit_2026-10-19_wave4_report.md`;
  `.planning/PLAN.md` *Phase 6* and *Remaining P1 debt*;
  `.planning/ARCHITECTURE.md` (the error-code comparison sentence);
  `.planning/archive/mcp_surface.json`; `docs/RELEASING.md`; commits
  `3dc9e42..976dc2e`, `27ae582`, `cf1475e`, `4b765e1`.
- **Reversible:** the external-use reductions can be re-opened, but a
  re-publicized item needs a documented external caller; the notices fixes
  must stay (see `docs/RELEASING.md`).

## 2026-10-19 — Wave 3 gate passed on the integrated tree

- **What:** the integrated full gate at `2dbd703` + `d669848` passed: fmt,
  clippy, deny, machete, source limits, duplicate checks, producer coverage,
  governance, and version; **1,176 tests passed, 0 failed, 1 ignored**;
  coverage **93.27%** (11,661 / 12,503; gate 83%); performance 0.97x–1.06x
  against the 20% threshold; **0 production files over 200 code lines**. The
  only remaining clippy `too_many_lines` warning is a test function inside an
  inline test module in `tools/report_completeness.rs`, not production code.
- **Why:** the wave-3 landed entry below recorded only per-package library
  suites and explicitly left the integrated gate pending. This entry records
  its outcome so a later reader neither re-runs it to learn the answer nor
  mistakes the structural split for an unvalidated one.
- **Where:** `.planning/archive/audit_2026-10-19_wave3_report.md`;
  `.planning/PLAN.md` *Phase 5*; commits `0374d60..2dbd703`, `d669848`.
- **Reversible:** n/a — this records a gate result, not a design choice.

## 2026-10-19 — Wave 3 landed: all 40 production R4 entries split

- **What:** the seven wave-3 commits (`0374d60..2dbd703`) split every
  production R4 entry from the strict audit (40 of 109): 18 files above 200
  code lines, 17 functions above 50 lines, and 5 functions with cyclomatic
  complexity above 15. The production `slugaudit-line-exception` annotations
  were removed; `check_source_limits` passes on the files themselves. The
  splits added 21 internal modules: `connect/config_file.rs`,
  `connect/registry_types.rs`, `evidence/bindings.rs`, `graph/rust_paths.rs`,
  `project/gitignore.rs`, `server/handlers.rs`, `store/connection_guards.rs`,
  `sync/hash_io.rs`, `sync/manager_health_support.rs`,
  `tools/finding_insert.rs`, `tools/health_phase.rs`,
  `tools/report_envelope.rs`, `tools/report_sections.rs`,
  `tools/tool_counters.rs`, `util/capped_read.rs`, `util/style.rs`,
  `util/time.rs`, `watch/errors.rs`, `watch/manager/events.rs`,
  `watch/manager/scopes.rs`, and `watch/scope_apply.rs`. The full library
  suite was green in each of the seven packages. Wave 3's integrated full
  gate is **pending** at this record; this entry does not claim it passed.
  One stale `slugaudit-line-exception` comment remains at
  `src/tools/health.rs:33` (108 code lines, passes without it); it is source
  cleanup, not a gate failure.
- **Why:** R4 was the production half of the remaining P1 debt. The refactor
  is structural: cohesive helpers move into modules so every file and
  function meets the R4 targets, without changing tool request/response
  schemas or runtime behavior. Keeping an annotation on a file that now
  passes without one would leave a false claim at the annotation site.
- **Where:** commits `0374d60`, `8a4db15`, `dad37b0`, `00943f5`, `44d19d9`,
  `2ebddb0`, `2dbd703`; `.planning/MODULE_MAP.md`;
  `.planning/PLAN.md` *Phase 5*.
- **Reversible:** the commits are revertible; the split is not a reason to
  re-inline the modules. Wave 4 (R8 public contract) is next and is **not
  started**; wave 5 is **not started**.

## 2026-10-19 — Mutation-testing scope: five files, hash_io added for the moved hash functions

- **What:** the `cargo-mutants` step now covers
  `src/sync/revision.rs`, `src/sync/publish.rs`, `src/sync/hash.rs`,
  `src/sync/hash_io.rs`, and `src/tools/context.rs`. `hash_io.rs` was added
  because the wave-3 R4 split moved `hash_file` and `stat_identity` out of
  `hash.rs` into it without changing them; the other three files are
  unchanged. This is not a widening of the correctness core — it is the same
  functions at their new path.
- **Why:** the scope exists to mutate revision identity, atomic publication,
  CAS/change detection, and the freshness-verification entry point. Keeping
  only `hash.rs` after the split would mutate hash types and in-memory
  digests while leaving the bounded streaming read and stat identity
  unmutated. The measured widening cost (13x / 21x for all of `src/sync/**`
  or `src/tools/**`) still argues against covering more files, not for
  dropping the moved ones.
- **Where:** `.github/workflows/quality.yml` (the step comment and the five
  `--file` flags); `AGENTS.md` *Testing and QA → Mutation testing*;
  `.planning/PLAN.md` *Accepted and held decisions*; commit `8a4db15` for the
  split.
- **Reversible:** yes, but a file leaving the correctness core needs a new
  decision and must be updated in the workflow comment, AGENTS.md, and the
  plan together.

## 2026-10-19 — Wave 2 landed; final gate passed on `main` at `6728735`

- **What:** wave 2 closed the six production-state gaps commissioned after
  wave 1. The integrated tree at `6728735` (base `6241b43`) passed the full
  gate: `cargo fmt --all -- --check`; `cargo clippy --all-targets
  --all-features --locked -- -D warnings`; **1,174 tests passed, 0 failed, 1
  ignored**; `cargo deny check` (one unmatched license exception warning);
  `cargo machete`; **93.10%** line coverage (11,341 / 12,181; gate 83%);
  `check_source_limits`; `check_no_duplicates`; `check_producer_coverage`;
  `check_governance`; `check_version_consistency`; `check_docs_drift`;
  `check_mcp_surface`; and `check_performance` at 0.98x–1.14x against the
  20% threshold. All pass.
- **Why:** the wave-1 tree could not pass `check_docs_drift` or
  `check_mcp_surface` and carried a report walk, a closed-pipe CLI panic,
  untested defensive arms, and a stale baseline. The final gate is the
  acceptance evidence for closing them; a future reader should not have to
  re-run the wave to learn the outcome.
- **Where:** `.planning/archive/audit_2026-10-19_wave2_report.md`;
  `.planning/PLAN.md` *Phase 4* and *Final gate*; commits `7a782de`,
  `326ec9a`, `5d77856`, `b19f4a0`, `bac3d0a`, `1b2b671`, `ed58a4b`,
  `6728735`.
- **Reversible:** n/a — this records a gate result, not a design choice.

## 2026-10-19 — Persist the unrepresentable-path count; schema version 7 → 8

- **What:** `b19f4a0` added a disposable
  `unrepresentable_paths (path BLOB PRIMARY KEY)` table and bumped
  `CURRENT_SCHEMA_VERSION` from 7 to 8. Paths are raw bytes because a non-UTF-8
  name has no text form. Full-publish discovery
  (`src/sync/publish_attempt.rs`) and the healthy-path stat sweep
  (`SweepReport.unrepresentable`, persisted in `src/sync/manager_health.rs`)
  replace the set exactly, including removals; the public reconcile/drain
  paths persist the sweep's set too. `report` now reads
  `SELECT count(*) FROM unrepresentable_paths` inside its verified read
  transaction and no longer calls `sync::discover`. Tests cover add → 1 →
  remove → 0 through both full and watcher-backed syncs. `index_partial`
  remains `count > 0 || skipped > 0`.
- **Why:** the report was doing a bounded filesystem walk on every read path to
  count paths the index cannot store. The count is derived, disposable state,
  so persisting it keeps the database disposable (it is recomputed by sync) and
  removes I/O from a read. A schema bump is the documented discard-and-rebuild
  path, so an existing version-7 database rebuilds on first use; no migration
  code is written.
- **Where:** `b19f4a0`; `src/store/schema.sql`;
  `src/store/migrations.rs`; `src/sync/publish.rs`
  (`replace_unrepresentable_paths`); `src/sync/publish_attempt.rs`;
  `src/sync/sweep.rs`; `src/sync/manager_health.rs`;
  `src/tools/report.rs`; `src/tools/report_completeness.rs`.
- **Reversible:** the table can be dropped with another schema bump; the
  public response fields (`unrepresentable_path_count`, `index_partial`) stay.

## 2026-10-19 — CLI output is fallible; a closed pipe is a clean exit

- **What:** `bac3d0a` + `1b2b671` added `cli::out`, `cli::err`, `write_to`, and
  `is_broken_pipe`. Every production `print!`/`println!`/`eprint!`/`eprintln!`
  in the CLI, connect, install, and update files was replaced with a fallible
  write and flush. A `BrokenPipe` anywhere in the error chain exits with status
  0 and no error log; any other write failure propagates through the module's
  typed error. `ConnectError::Io` and `InstallError::Io` expose their I/O error
  through `source()` (Display unchanged) so the chain walk can classify it.
  End to end: menu output piped into `head -c 10` exits 0 with empty stderr,
  and `slugaudit help >/dev/full` exits 1 with the I/O error.
- **Why:** a reader that closes the pipe has gone away; that is not a command
  failure, and the previous `print!`/`println!` panicked on `EPIPE`. Other
  write failures are real failures and must still be reported. The typed-error
  `source()` change keeps the cause without changing user-facing messages.
- **Where:** `bac3d0a`, `1b2b671`; `src/cli.rs`; `src/main.rs`;
  `src/connect.rs`; `src/connect_exec.rs`; `src/install.rs`; `src/menu.rs`;
  `src/prompt.rs`; `src/update.rs`; `src/lib.rs`.
- **Reversible:** do not restore panicking writes; a new CLI output site must
  use `cli::out`/`cli::err` or return its module's typed error.

## 2026-10-19 — Failure-arm disposition: remove the dead, retain the live, accept the rest with a reason

- **What:** `ed58a4b` closed the wave-1 unreachable-arm list:
  - `SyncError::Revision` and `SyncError::Read` **removed** as provably
    unused.
  - `DiscoveryError::Walk` **retained**: it is constructed in
    `src/sync/discovery.rs`'s skip path (`DiscoveryError::Walk(error)`), so
    the wave-1 premise that it was dead was wrong.
  - The connection `Configure` arms for `busy_timeout` and function
    registration **accepted as unreachable/documented**: they fail only on
    `SQLITE_NOMEM`/`SQLITE_MISUSE` from a NULL or invalid handle, which the
    safe rusqlite API cannot produce hermetically; the 5-second timeout
    behavior remains tested.
  - The `finding` u32 line-count conversion and the `finding_read`
    query-map bind failure remain **accepted** from wave 1 as unreachable for
    storable content / valid bound parameters.
  - Tested through pure helpers rather than production hooks: install's
    `CurrentExe`/`Clock`, connect's `BinaryPath`, prompt flush, and update's
    platform predicate (`ensure_platform_supported`).
  - Tested through clean refactors: the stale-metadata discard path, the netfs
    positive rejection (`reject_if_network(path, on_network)`), the context
    read-transaction start, and project-control's discard/reopen path (its
    serialize arm was removed by constructing the value directly).
- **Why:** dead code should go; live code must not be removed because an audit
  thought it was dead; a defensive arm that cannot be reached without an
  invalid handle or OOM stays, documented at the site, rather than growing a
  production-only test hook. No lint suppression was added.
- **Where:** `ed58a4b`; `src/sync/manager.rs`; `src/sync/discovery.rs`;
  `src/store/connection.rs`; `src/store/netfs.rs`;
  `src/tools/context_transactions.rs`; `src/tools/project_control.rs`;
  `src/tools/finding.rs`; `src/tools/finding_read.rs`; `src/install.rs`;
  `src/connect.rs`; `src/update.rs`.
- **Reversible:** re-add a removed variant only with a constructor and a test;
  changing an acceptance needs a new decision entry.

## 2026-10-19 — Fresh docs, generated surface and baseline, and durable audit evidence

- **What:** the canonical docs were authored fresh at `7a782de`; nothing was
  restored from the deleted 2026-10-18 planning paths. `326ec9a` corrected
  AGENTS.md's parser-pack location to `src/parse/mod.rs`'s `pub const
  PACK_VERSION`. The MCP surface was recorded from the integrated tree at
  `5d77856` (8 tools, 8 resources), and the performance baseline at `6728735`
  (12 benchmarks: discovery 4, parsing 4, search 4) using the default
  `target/` because `check_performance` hardcodes `target/criterion`. The four
  audit records were copied as **new** 2026-10-19 files under
  `.planning/archive/`: `audit_2026-10-19_report.md`,
  `audit_2026-10-19_ledger.md`, `audit_2026-10-19_wave_report.md`, and
  `audit_2026-10-19_wave2_report.md`.
- **Why:** the docs must describe current behavior, not a deleted record.
  Generated artifacts are reviewable evidence only if they are emitted by the
  tree that passed the gate, and audit evidence in `/tmp` disappears with the
  session. Copying it into the repository makes every count in `PLAN.md`
  re-checkable.
- **Where:** `7a782de`, `326ec9a`, `5d77856`, `6728735`;
  `.planning/archive/mcp_surface.json`;
  `.planning/archive/perf_baseline.json`; `.planning/archive/audit_2026-10-19_*`.
- **Reversible:** the surface and baseline are re-recordable after a deliberate
  change; the copied audit records are historical and must not be edited or
  deleted.

## 2026-10-19 — Wave 2: fresh canonical docs, recorded surface, persisted report count, closed-pipe handling, failure arms, new baseline

- **What:** the wave-2 packages close six production-state gaps:
  1. the canonical docs are authored fresh from current behavior
     (`.planning/PLAN.md`, this file, `docs/CLIENTS.md`,
     `docs/TROUBLESHOOTING.md`, `docs/RELEASING.md`, `docs/about.toml`,
     `docs/third-party-notices.hbs`), never restored from git;
  2. `.planning/archive/mcp_surface.json` is re-recorded from the live
     `tools/list` + `resources/list` surface;
  3. `report` reads the non-UTF-8 path count persisted at sync instead of
     walking the filesystem on every call;
  4. CLI writes are fallible and treat `BrokenPipe` as a clean exit rather
     than panicking;
  5. the remaining unreachable failure arms are tested through a clean
     refactor, explicitly accepted, or removed — with no production-only test
     hooks and no lint suppressions;
  6. a fresh performance baseline is recorded after the code changes.
- **Why:** the wave-1 tree was honest about its missing inputs but could not
  pass `check_docs_drift` or `check_mcp_surface`; and the wave report itself
  named the report walk, the closed-pipe CLI panic, the untested arms, and a
  stale baseline as residual production-state gaps.
- **Where:** `.planning/PLAN.md` *Phase 4*; `/tmp/audit3/WAVE_REPORT.md`
  *Residual notes*; `src/bin/check_docs_drift/canonical_docs.rs`;
  `src/bin/check_mcp_surface/main.rs` (`SNAPSHOT_PATH`);
  `src/tools/report.rs`; `.github/workflows/quality.yml` (baseline gate is
  local-only).
- **Reversible:** each item individually, but re-opening the deleted docs is
  not a reversal — see the 2026-10-19 entry below.

## 2026-10-19 — Delete the superseded planning record and author the canonical docs fresh

- **What:** `6241b43` deleted the accumulated `.planning/**` and `docs/**`
  planning record. The surviving set is the fixed canonical list in
  `src/bin/check_docs_drift/canonical_docs.rs`, and every surviving or
  replacement document is written from current behavior. No deleted document
  is restored, copied, or recovered from git history.
- **Why:** the old record had grown into many overlapping plans and audit
  ledgers whose claims drifted from the code. A next reader could not tell
  which file was current. The fix is one small, fixed set of documents whose
  safety comes from a gate (existence, module map, test count), not from
  volume. Restoring a deleted file would re-import the drift.
- **Where:** `6241b43` (`docs: remove the superseded planning records`);
  `src/bin/check_docs_drift/canonical_docs.rs`;
  `.planning/PLAN.md` *Documentation policy*.
- **Reversible:** no. Reversing means re-establishing the old record and
  defeating the deletion's purpose. A historical fact that matters belongs in
  a new decision entry or in `AGENTS.md`'s session-hygiene section, not in a
  restored file.

## 2026-10-19 — Wave 1: fix the 55 production findings, not the test-side counts

- **What:** wave 1 fixed the 55 confirmed production findings from the
  2026-10-19 strict audit (R1 3, R2 1, R6 22, R7 5, R10 3, R12 21), including
  the top-10 CAS-baseline defect in `src/sync/manager_health.rs`. It added 126
  tests (1,030 → 1,156), raised line coverage 90.26% → 92.85%, and reported
  the three dead variants (`SyncError::Revision`, `SyncError::Read`,
  `DiscoveryError::Walk`) rather than writing tests that cannot fail. The
  test-side R1/R2 counts from the audit were not burned down.
- **Why:** the crate's lint contract denies `unwrap`/`expect` outside
  `cfg(test)` because a test panic *is* the failure signal. Rewriting 1,918
  test panics to lower an audit count would make tests worse, not the
  product better. Production correctness was the target; R4/R5/R8 were left
  as tracked debt rather than declared fixed.
- **Where:** `/tmp/audit3/WAVE_REPORT.md` (commit map and final gate);
  `.planning/PLAN.md` *Phase 3* and *Remaining P1 debt*;
  `AGENTS.md` *Lint contract*.
- **Reversible:** the code is revertible; the disposition is not a reason to
  redo the test-side counts.

## 2026-10-19 — Revert the static grammar bundle; keep runtime grammar downloads

- **What:** the hybrid static bundle — 21 evidence languages compiled into the
  binary at `2ca94a8`, trimmed to 14 core grammars at `024ec1e`, merged at
  `26ed140` — was reverted at `f9544ca` (`revert(parse): remove the static
  grammar bundle and return to runtime downloads`). The product again takes
  grammar availability from the exact-pinned
  `tree-sitter-language-pack = "=1.18.0"`, whose runtime downloads were
  already recorded as an accepted risk at `fe181df`. `parse::PACK_VERSION`
  (`src/parse/mod.rs`) remains the version recorded with every revision.
- **Why:** the static bundle was a second distribution path beside the pack:
  a grammar's availability could disagree with the pack pin, and every pack
  bump would need the bundle re-derived. One pinned source of grammar truth is
  easier to verify than two. The accepted risk of runtime downloads was
  already written down and is unchanged.
- **Where:** `f9544ca`; `Cargo.toml` (`tree-sitter-language-pack = "=1.18.0"`);
  `src/parse/mod.rs`; `src/bin/check_version_consistency`;
  `AGENTS.md` *Runtime and tooling preferences*.
- **Reversible:** only with a new decision and owner approval. Re-proposing
  the bundle must name what pins it to the pack version and how a pack bump
  updates it.

## 2026-10-19 — `max_total_import_bytes` is a work budget, not a memory bound

- **What:** `max_total_import_bytes` caps the sum of sampled file sizes in one
  publish. It is explicitly a **work budget**: peak process memory also
  includes in-flight samples, evidence payloads, and parse transients, so a
  tree can fit the budget and still exceed available RAM. A separate
  memory/in-flight bound is a pending architectural decision, not something
  the existing limit pretends to provide. Per-file and query-value limits are
  clamped to `SQLITE_MAX_VALUE_BYTES` (1,000,000,000, the bundled SQLite's
  `SQLITE_MAX_LENGTH`), because a larger value cannot be inserted; an
  over-limit file becomes a reported skipped row instead of a failed publish.
- **Why:** honesty about what a cap bounds. Calling the import budget a memory
  bound would make an OOM look like a broken limit. The SQLite clamp was
  verified against `libsqlite3-sys` 0.35.0 on 2026-10-19.
- **Where:** `src/model/limits.rs` (`ResourceLimits::max_total_import_bytes`
  doc comment and `SQLITE_MAX_VALUE_BYTES`);
  `src/model/profile.rs`; `ARCHITECTURE.md` *Store and schema lifecycle*;
  `.planning/PLAN.md` *Phase 2*.
- **Reversible:** the clamp is required by the storage engine; the work-budget
  classification changes only if a real memory bound lands and covers the same
  path.

## 2026-10-19 — Audit triage: production findings first, counted evidence second

- **What:** the 2026-10-19 strict audit read all 259 committed `.rs` files
  (50,325 lines) against R1–R12 and re-checked every cited violation in a
  separate pass: 2,550 confirmed, 27 dropped on re-check, 75 suspected misses
  recorded in Appendix C but not counted. Triage split production entries from
  test/bench/fuzz and dev-gate entries, and wave 1 fixed the production set.
  R4 (109 entries, 40 production), R5 (142, 110 production) and R8 (258
  production) were carried as debt rather than mass-edited.
- **Why:** a count that mixes test panics (exempt by contract) with production
  panic risks is not a priority list. The audit's per-rule scope column is what
  makes its numbers quotable; treating the raw total as a defect count would
  send a later wave after test code.
- **Where:** `/tmp/audit2/AUDIT_REPORT.md` (rule table, §5 top 10, Appendix B,
  Appendix C); `.planning/PLAN.md` *Remaining P1 debt*.
- **Reversible:** n/a. Re-auditing is always allowed; changing the rule table's
  scope definitions needs a new entry.

## 2026-10-16 — Path-logging policy: redacted ids for correlation, raw paths only in one-shot diagnostics

- **What:** a server log line that can be correlated across a session must not
  carry a raw filesystem path. Paths can contain usernames, repository names,
  and other private data, and stderr is always on under an agent session.
  Correlation sites emit `util::redacted_path` — a stable BLAKE3 hex id — so
  the same path is still recognizable across lines. Three kinds of site may
  emit a raw path: a one-shot diagnostic whose whole purpose is to name the
  file just operated on (the staged-binary cleanup in `install_place.rs`, the
  permission repair in `project/activation.rs`, the leftover temp file in
  `connect_exec.rs`). SQL text, finding titles/descriptions, indexed source
  content, and environment values never enter logs at all; `ErrorData.data`
  carries only static codes and remedies. The contract is pinned by
  `src/tools/redaction_tests.rs`.
- **Why:** the alternative — logging raw paths everywhere — leaks private data
  into a stream the agent host often retains. The alternative — redacting
  everything — makes a one-shot fix instruction unactionable. The policy
  names the boundary instead of leaving it to each call site's taste.
- **Where:** `src/util.rs` (`redacted_log_id`, `redacted_path`);
  `src/tools/redaction_tests.rs`; `src/sync/publish_tests.rs` (redacted path
  assertions); `src/error_codes.rs` (static remedy rule);
  `ARCHITECTURE.md` *The tool-call log*.
- **Reversible:** yes in code, but do not: the tests exist because this was
  regressed before. A new one-shot exception must say why the raw path is
  required and must not sit on a correlation path.
