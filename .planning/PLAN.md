# Plan

Branch `googlefi-code`. **This is the single active plan.** It absorbs and replaces
`AGENT_EXPERIENCE_PLAN.md` and `PROD_READINESS_PLAN.md`, both deleted on 2026-09-30 when
this file was written.

## Status

**Landed and validated (2026-09-30) — three workstreams; the full gate suite is green**
(`cargo fmt --check`, clippy `-D warnings`, **556** lib tests plus every integration crate,
`check_source_limits`, `check_no_duplicates`, `check_docs_drift`):

1. **Phase 1 — Stop lying.** All four items in; `CURRENT_SCHEMA_VERSION` went to `4` here and is
   `6` as of Phase 3 (`src/store/migrations.rs`). What the independent review changed in the same round —
   and what it deliberately left — is recorded under *Review findings and their
   disposition*; read that before touching Phase 2.
2. **Documentation reset.** The canonical set is `README.md`, `AGENTS.md`,
   `.planning/ARCHITECTURE.md`, `PLAN.md`, `.planning/archive/DECISIONS.md`,
   `docs/CLIENTS.md`, `docs/TROUBLESHOOTING.md`. ~5,600 lines of drifted docs were deleted,
   ~2,300 written, and the 1,287-line decision log kept. `check_docs_drift` was rewritten
   to enforce that set — its old `IMPLEMENTATION_PLAN.md` §22/descope checks died with the
   file they policed. The lost-facts audit that followed recovered seven dropped facts and
   caught the old `AGENTS.md` claiming a `query` SQL keyword blocklist that does not exist,
   plus a `query` description listing two `dependency_edges` columns that do not exist.
3. **Connect surface rebuilt and verified.** Client configs now resolve against `$HOME`
   instead of `~/.slugthug` (they were being written where no client reads them); the
   registry models three states (`CliDirect` / `CliInteractiveOnly` / `ConfigFile`) with
   per-entry `ConfigShape`, `ConfigPath`, `Evidence`, `reference` and `docs_url`; **18 of
   26 clients are `VerifiedByProbe`**; clients that were being called with subcommands that
   do not exist (`agy`, `kimi`, `crush`, `goose`) now use the documented route or refuse
   **with a paste-ready remedy, the version the advice came from, and a docs link**;
   `connect` no longer runs a client's `remove` before `add`; `docs/CLIENTS.md` is derived
   from the registry rather than hand-written.

4. **Phase 0 — the instrument.** Every tool call now carries a process-local `call_id` in its
   span alongside the session id, and emits one greppable completion line with the tool name,
   duration, outcome, and the `ToolCounters` deltas for that call (deltas, never absolutes —
   the counters are process-wide and cannot be reset). A session's call sequence is recoverable
   from stderr, demonstrated against the real binary, which logged `report` → `report` → `query`
   as calls 1, 2, 3 with durations. The redaction invariant is re-tested: the new line carries
   no arguments and no results. Both recovery commands are in `ARCHITECTURE.md`.

5. **Phase 2 — the door.** All four items landed. Item 1 rewrote `INSTRUCTIONS` (above); item 2
   gives every discovered file a row, so a skipped file is countable instead of invisible, and
   `report` carries a completeness block with a partial-index flag; item 3 adds six
   machine-readable error codes with static remedies, so a caller can branch — the sync-budget
   failure no longer tells a caller to retry something permanent; item 4 makes the not-enabled
   state one actionable instruction behind all eight tools, with `health` answering normally
   when nothing is enabled.
6. **Phase 3 — recall at scale.** All nine items landed: `leads` enumerates to completion against
   a byte budget, counts completeness in files, declares per-mechanic coverage (`rust_only` /
   `all_languages`) instead of letting a zero read as clean, filters by path, and refuses a
   resumed page pinned to a stale revision; three producer-less evidence kinds are deleted and a
   new gate refuses to let another appear; and `report` can name generated and vendored files.
7. **Phase 4 — the contract (items 1–3).** Output schemas, titles and annotations on all eight
   tools (annotations derived from the crate source, not guessed); the payload-key reference moved
   out of `INSTRUCTIONS` into a resource, shortening the briefing by 337 characters; a
   `check_mcp_surface` gate that derives the live surface in-process and refuses on drift unless
   `--record` is passed; blast radius as arithmetic in `report`; and an optional `mode:
   full|compact` on `leads`, where `compact` without a mechanic returns the whole mechanical
   surface as `mechanic, file, line` triples in one response — affordable precisely because the
   `detail` text is what the page budget was mostly spent on.
8. **Phase 5 — the loop (item 1).** `finding` dedupes a restated conclusion inside the session by
   (path, source_hash, line range, category, title), updating the row in place and keeping its id,
   while a restatement after the file changed correctly becomes a new finding. Items 2 and 3 are
   the plan's own non-goals.

**Committed (2026-09-30).** One commit per workstream, each leaving the full gate suite green:
`ec6a464` (the deviations log), `ecc25e7` (Phase 1), `9c0f77f` (the connect registry — its
`ARCHITECTURE.md` rewrite lands there rather than with the documentation reset, because the module
map names the new connect modules and the `*_tests.rs` count sentence is gate-checked against the
new test file), `dca4910` (the documentation reset), `5302bf0` (Phase 0's instrument), `9085f6c`
(the plan/deviations bookkeeping), `b9bfb39` (Phase 2's index completeness — `fmt`, `clippy
-D warnings`, 565 lib tests and the three cheap gates verified green on it), `fb6eea5` (Phase 2's
error surface), `9ec589b` (Phase 3), `6433b06` (the log-capture harness fix), `869c126` (the
offset and `char_column` follow-ups), `2b64cc3` (store directory permissions), `ec10812` (the
governance shell), `afe4128` (the hermetic connect suite), `d9cb506` (finding dedupe),
`8c03f9e` (the MCP surface contract), `5a59e69` (the install/update rollback), `210ecaa` (the
adversarial `query` tests), `ca6c569` (the governance and version gates) and `a7f327c` (the
compact `leads` mode). **Not pushed** — the branch has no remote counterpart and publication is
the owner's call.

**Not started:** the *Connect surface* follow-up; the credential-content policy and a private
conduct/security contact (owner decisions); the workspace split (which *Sequencing* puts last,
because it renames every path); and one thing that cannot be done from here — the release
workflow has never executed, because that needs a tag push and publication is the owner's call.
Everything else in this backlog has landed.

Item 4's optional tool-hiding half is deliberately *not* implemented — see its entry for why. Note
for whoever edits this file next: several agents wrote to it concurrently on 2026-09-30 and the
status block briefly contradicted itself as a result — treat the commit log as authoritative when
the prose and the hashes disagree.

---

## Why this plan exists

The product was founded on an observation, and that observation is the only success
criterion this project has ever had:

> A large audit was underway. SQLite was suggested as a way to stop re-reading files.
> The agent tried it, then **kept calling it — call after call, unprompted — and
> afterward said, in its own words, why it helped.** Sessions where that happened also
> **found issues that had been missed** and **finished audits faster with the extra
> results**.

That is the target: **an agent that lights up the screen and says why.** Voluntary
repetition inside one session, triggered by live pain, confirmed afterward. Token
savings are a byproduct. Tone is not the product. **Not missing an issue is the
product.**

This metric was never written down anywhere in the repository, so each subsequent AI
author optimized a proxy it invented for itself — audience framing, page caps, honesty
in the pitch — which is how the surface drifted away from the thing that worked. This
document exists so the next author has the real target in front of it.

## Non-negotiables (do not trade these away)

1. **Evidence is not judgment.** No risk scores, no severity, no ranking, no
   "priorities". Findings remain agent-authored. This is what makes every number
   quotable; a count carrying a verdict becomes arguable and therefore useless.
2. **Freshness without re-verification.** Sync before answering stays automatic.
   An index an agent must double-check has no value.
3. **No silent truncation, anywhere.** Every cap, skip, and producer gap is either
   reported in the response or it is a defect.
4. **Every count must be quotable as-is.** If a number needs a caveat the response
   doesn't carry, it is wrong.
5. **No new surface may become the sole holder of a fact.** The DB stays disposable.

## Guiding design rule

An agent handed a new tool is not neutral; the rational prior is *"another one that does
less than what I already have."* That prior is broken by exactly one thing: **call #1
returning something true the agent did not ask for and could not have cheaply learned.**
A first response that returns exactly what was requested confirms the prior. Every
change below is judged against that rule.

---

## Phase 0 — Baseline and instrument (landed 2026-09-30)

**Goal:** know the current state and be able to see adoption move.

1. Record the gate suite output so later regressions are attributable. **Recorded 2026-09-30**,
   on the 12-core Ryzen 9 5900X this plan's numbers come from: seven CI gates PASS
   (`check_source_limits`, `check_no_duplicates`, `check_docs_drift`, `check_producer_coverage`,
   `check_mcp_surface`, `check_governance`, `check_version_consistency`); 630 library tests and
   every integration crate green (the six largest: 24, 11, 10, 8, 7, 7); `fmt` and
   `clippy -D warnings` clean; 36 concurrent library-suite runs with zero failures (the harness
   is deterministic under load — see the log-capture entry in `DEVIATIONS.md`);
   `check_performance` and `check_coverage` are local-only, and the latter's threshold is 83
   (line coverage was **not** re-measured — `cargo-llvm-cov` is not installed on this machine).
   The two machine-readable halves of this baseline already live where a consumer reads them:
   `.planning/archive/perf_baseline.json` (`check_performance`) and
   `.planning/archive/mcp_surface.json` (`check_mcp_surface`).
2. Add a per-call correlation id to the tracing span in `src/server_runner.rs` —
   `info_span!("tool_call", tool = …)` today, `src/server_runner.rs:59` — and log the
   session's call sequence: tool names, order, tool-call deltas from `ToolCounters`
   (`src/server_runner.rs:112`).
3. Ground truth for the replay: this plan's acceptance tests are *behavioral*, so the
   sequence log is the instrument, not the tests.

**DoD:** a session's call sequence is recoverable from stderr.

## Phase 1 — Stop lying (landed and validated 2026-09-30)

A wrong answer costs more than a missing one; each of these produced a false verdict.
All four are in, each with a test that fails before the change.

1. **`known_paths` from discovered paths, not indexed rows.** `known_paths` now starts
   from the discovered set and unions the stored paths (`src/sync/revision_edges.rs:26`),
   so an import targeting a discovered-but-skipped file resolves instead of
   `Unresolved` (which an agent reads as *unused / dead* — a fabricated finding).
2. **`ExtractionCompleteness::ContentOnly` is emitted and `Full` is reachable.**
   Content indexed but not parsed now reports `ContentOnly`
   (`src/sync/analyze.rs:40`); `Full` is downgraded to `Partial` when evidence caps
   discarded records (see review findings).
3. **The detail clip is visible.** `MAX_DETAIL_CHARS = 200`
   (`src/tools/leads.rs:28`) no longer clips silently: `LeadRow.detail_clipped` is set
   only when characters were removed (`src/tools/leads.rs:91-100`).
4. **`evidence.kind` is `CHECK`ed** alongside the parser columns it sits beside
   (`src/store/schema.sql:84`). The schema version bumped; the mismatch path discards and
   rebuilds, so it was free. The test derives the variant list from the enum itself, so a
   12th variant cannot slip past it.

## Correctness follow-ups (small; identified 2026-09-30, not scheduled)

Found while auditing the surfaces and verified in the code. **All five are resolved**
(2026-09-30). Each was independently small, and none is covered by the phases below.

1. **`leads` with `limit: 0` can page forever — resolved 2026-09-30** (the row cap is gone; `limit` is clamped to `>= 1`, and the response reports `limited_by`/`complete`, so a resumed page always advances). `limit` is only floored at the maximum
   (`unwrap_or(100).min(500)`), so `LIMIT 0` returns an empty page while `truncated` stays
   `true` and `next_offset` equals the offset it was given — a caller following the
   documented paging contract loops on the same page forever. Clamp to `>= 1`, or reject
   it as a contract error. (`src/tools/leads.rs`, `src/tools/leads_query.rs`.)
2. **`query.offset` is unbounded — resolved 2026-09-30** (`869c126`). It was a `usize` interpolated
   straight into `OFFSET {offset}`, so a value above `i64::MAX` was accepted, coerced to a REAL by
   SQLite, and interpreted as a different number instead of returning a contract error — a caller
   paging a result set got the wrong page with no error. It is now refused before the wrapper is
   built, with the bound named in the message, and a test asserts both the refusal and that the
   bound itself is still a usable offset. `leads` never had this problem: it binds its offset as a
   parameter. (`src/tools/query.rs`.)
3. **`char_column` can panic on a non-boundary offset — resolved 2026-09-30** (`869c126`). It sliced
   `source[..byte_offset]` on a doc-comment-only precondition, so a Tree-sitter node boundary
   landing mid-codepoint panicked inside `structure` on real project content — a tool call that
   dies mid-audit. An offset landing mid-codepoint, or past the end of the source, is now floored
   to the nearest boundary, and the doc comment states the guarantee rather than the precondition.
   (`src/model/span.rs`.)
4. **Tests execute real client binaries — resolved 2026-09-30.** `AgentDef::is_connected` falls
   through to `which(cli)` + `cli_check_connected(cli)`, so a connect test that passed a
   temporary `home` still ran whichever client CLI happened to be installed: `codex` rewrote the
   user's `~/.codex/config.toml` during a test run on 2026-09-30 (content intact, no slugaudit
   entry). No production change was needed — the tests were the defect. `tests/connect_tests.rs`
   now runs the real binary against a temporary home with `PATH` replaced by a fake CLI per agent
   that records its argv, and the dialect matrix is asserted from that log (stricter than the
   config inspection it replaced); the self-skip and the ⚠️ warning are gone. `src/connect_tests.rs`
   swapped its PATH-*prepending* helper for replace/empty ones — prepending left real CLIs
   reachable further along — and `tests/menu_tests.rs` is sandboxed for the same reason. Verified
   by hashing the real `~/.codex/config.toml` and `~/.claude.json` before and after a full run:
   byte-identical.
5. **The log-capture tests lost their own lines — resolved 2026-09-30** (`6433b06`). The suite
   flaked under load (~3 suite-runs in 36) and always reported nine failures instead of one,
   because the panic poisoned `SESSION_TEST_LOCK`. The visible symptom — a capture window holding
   a `call_id` the test never made — was first read as another test's line landing in this one,
   and instrumenting the capture writer falsified it (0 of 246 writes came from another thread):
   the capture was losing lines, not gaining them, because tracing's callsite-interest cache and
   max level are process-global and `Rebuilder::JustOne` blind-caches `NoSubscriber`'s
   `Interest::never()`. Fixed in `store::test_capture` with a permanently registered permissive
   backstop, plus per-test tool literals for the count assertions and liveness controls where the
   assertion is absence-only. Differential: 3/36 failing suite-runs before, 15/36 with the literal
   scoping alone (insufficient), 0/108 and 0/48 after both. The harness contract is now written
   down in `AGENTS.md` and `.planning/ARCHITECTURE.md`.

## Audit follow-ups (identified 2026-10-16)

From a full code-quality audit of all 227 `.rs` files (run against `57c3caa`).
The full ledger — every site with its quoted source line, plus the detector
commands and their exact counts — is `.planning/archive/audit_2026-10-16.txt`. It is the
evidence behind the items below; this section is the list. Items here carry a
state.

### Re-audit at `4de8e9b` (same day) — full-rerun ledger; repaired 2026-10-16 (committed `a37113e`..`75e0fb6`)

Re-ran the complete audit (227 of 227 files; detectors plus context reads of every
flagged site) against the post-fix HEAD. Ledger: `.planning/archive/audit_2026-10-16_HEAD.txt`
(now carries its `# Disposition` block). The root `violation_ledger.txt` was archived as
`.planning/archive/violation_ledger_57c3caa_superseded.txt`.

1. `check_coverage` **FAILS at HEAD**: 81.84% < 83% gate — **resolved**: measurement scoped to shipped runtime (dev-only `src/bin/*` excluded per AGENTS.md; DECISIONS 2026-10-16); now 88.32% vs the unchanged 83 floor.
2. `fuzz/` workspace deny failures — **resolved**: lockfile rustls 0.23.45→0.23.46, path dep versioned, scoped NCSA license exception, deny steps added to `quality.yml` + `advisories.yml`; fuzz `cargo-machete` clean.
3. Invalid `SLUGAUDIT_PROFILE` silently ignored — **resolved**: `profile::profile_from_env` warns (test: `a_set_but_unknown_profile_warns_and_selects_nothing`); env-touching limits tests now take `TEST_ENV_LOCK`.
4. Silent `canonicalize` discards ×7 — **resolved**: `util::canonical_or_self` warns with `redacted_path`; all seven sites converted.
5. `update` spawn-error conflation — **resolved**: `UpdateError::ToolSpawn { tool, source }` carries the real io error at all three sites; display test updated.
6. R4 size debt — **partially resolved**: all six shipped CC>15 functions decomposed (re-measured with `cargo-crap` after the pass: zero shipped functions above CC 15 remain; the five survivors are all dev-only `src/bin/*` mains — `scan_step` 47, `check_performance main` 40, `check_mcp_surface walk` 19, `check_source_limits main` 18, `manifests check` 18; `publish_revision` split into private in-file steps to preserve the mutation scope) and every >100-line shipped function decomposed (`sample_all_with_deadline`→`Batch`, `sync_by_health`→branch methods, `try_publish`, `sweep`, `analyze`, `run_blocking`); `build_report` now 72 lines/CC ≤15. **Known debt, not scheduled: ~40 shipped functions of 51–100 lines.** File rows stand on the exception system (3 new annotations: `manager_health.rs`, `report.rs`, `publish_attempt.rs`).
7. R8 surface drift — **resolved**: 60 undocumented plain-pub items documented (rescan: 0); `json_contains_slugaudit`/`zed_contains_slugaudit` deleted; `reference_line`/`docs_line` made private; schema-type and composition-root "unused" items reviewed and kept pub (ledger disposition).
8. Unbounded foreign-file reads — **resolved**: `util::read_file_capped` (1 MiB, sample.rs pre-check + post-read re-check) at connect_exec ×2, connect_agents, install, and project/config.json (5th site found during the pass).
9. AGENTS.md stale lines — **resolved**: pack pin sentence now `=1.18.0` with a re-check note; leads shape rewritten to byte-budget (no row cap); coverage bullet states its scope.
10. R12 path-logging inconsistency — **resolved by rule** (DECISIONS 2026-10-16): redacted correlation ids, raw one-shot diagnostics with comments, project-relative ignore-file diagnostics.

### macOS: every database open was refused — resolved 2026-10-16

`src/store/netfs.rs`'s macOS branch ran `stat -f %T <db path>` before the database
existed, so the command failed on every first open and the guard failed closed: no
project could be indexed on macOS. BSD `stat -f %T` also prints the *file* type, not the
filesystem type, so the check never detected a network mount even when it ran. Every
`Rust quality` run on `main` failed on macos-latest (222 of 645 lib tests on run
38939256071), and that red was masked by the earlier clippy/coverage failures. Replaced
with a parse of `/sbin/mount` output (longest mount-point match over the nearest
existing canonical ancestor), as the pure `is_mount_table_network_filesystem`, which is
tested on every platform. State: **landed**; verified only by macOS CI, since the
macOS-only glue cannot compile on the Linux dev host.

### Symlinked parent directory refused every open — resolved 2026-10-16

`SQLITE_OPEN_NOFOLLOW` rejects a symlink in *any* path component
(`SQLITE_CANTOPEN_SYMLINK`, extended code 1550), not only the database file, so a
database path under a symlinked ancestor could not be opened. On macOS `/var` and `/tmp`
are links into `/private`, which failed 107 lib tests on macos-latest (run on `4085f5b`)
once the netfs fix above let them reach SQLite. Production roots are canonicalized at
activation, so real projects were probably unaffected; any non-canonical caller was not.
`src/store/connection.rs::resolve_parent_dir` now canonicalizes the parent before both
opens; the file itself stays guarded by `reject_symlink` and `NOFOLLOW`. Pinned on every
Unix by `a_database_under_a_symlinked_parent_directory_opens_for_both_modes`. State:
**landed**.

### Resolved 2026-10-16

1. **A deeply nested file aborted the whole server — resolved** (`a492f25`). The
   evidence walkers recursed once per parse-tree level, and tree depth is a
   property of file content, so a 12 KB Python file of 6 000 nested parens ran
   the thread stack out. A stack overflow is not a catchable panic: the process
   aborts and the caller's session dies. `MAX_WALK_DEPTH = 512` now bounds
   `walk_bindings`, `walk_imports`/`walk_imports_past`, `walk` and `collect` —
   the same bound the language pack applies to its own traversal. The cap is
   **reported**, not silent: `extract` returns whether a walker stopped early
   and `analyze` maps it to `ExtractionCompleteness::Partial`. Verified by
   re-running the reproducer: exit 134 → exit 0 with a normal report.
2. **`env::args()` panicked on a non-UTF-8 argument — resolved** (`3aca963`).
   argv is bytes on Linux, so a path completed from a Latin-1 filename reached
   `main` and exited 101 with a Rust panic — in the default `serve` entry point
   the MCP host launches, and in all nine `check_*` binaries. `args_os()` with a
   lossy conversion makes it a rejected command instead.
3. **JS/TS import edges were wrong — resolved** (`5208a94`). `f2c454b` gave
   `extract_python_reference` an `import` branch and dispatched it before the JS
   branch, so it claimed every `import` statement: `import x from 'foo.js'`
   extracted the binding `x`, and `import { b } from './b.js'` extracted `{`.
   `dependency_edges` for JS/TS resolved to nothing or to a name matching no
   file. Six tests failed on it.
4. **The suite's verdict depended on `NO_COLOR` — resolved** (`9d78d25`). The
   capture formatter coloured by default and emits escapes between a field's
   name, `=` and value, so `logs.contains("tool=\"probe\"")` matched nothing and
   three tests passed only with `NO_COLOR` set — green locally, red in CI.
   `with_ansi(false)`, plus a test asserting a control line IS present before
   asserting no escape byte.
5. **CI could not be green — resolved** (`1d64894`, `a539438`). Ten rustfmt
   hunks, two clippy warnings promoted by `-D warnings`, and two files over the
   line floor. The suite is now 643 passed / 0 failed with no environment
   variables set.
6. **`health`'s response and DB read — resolved** (`63434b7`). Four queries each
   degraded independently, so a response could carry a revision beside
   `file_count: -1`; `.optional().ok().flatten()` reported a broken database as
   an unpublished one. One `read_database()` now answers every field or none,
   and the response is grouped into `watcher`/`database`/`server` — the three
   categories the module doc already declared — so `-1` has nothing left to
   mean and `required` drops 10 → 3. `health.rs` 305 → 260 code lines.
7. **Invisible failures and unredacted paths — resolved** (`681eafd`,
   `2e3334d`). Failures that reached neither the caller nor a log: the watcher
   watch/unwatch logs sat *inside* the success branch; a sweep stat failure
   silently classified a file as deleted; an invalid `SLUGAUDIT_*` override was
   indistinguishable from unset; six sites printed absolute paths where
   `util::redacted_path` exists. Three messages asserted outcomes the code never
   established (`Wrote mcpServers.slugaudit` after a silently-skipped insert; a
   failed `mcp remove` reported as `mcp add`; "a newer release is available" on
   the arm where the comparison failed).
8. **Two gates passed having verified nothing — resolved** (`6763382`).
   `check_no_duplicates` turned every git failure into "skipping" and `main`
   still printed PASS; `check_docs_drift` check 3 returned silently if the prose
   claim moved. Both fail closed now.

### Re-verified 2026-10-16 against `850b09d` — items closed with evidence

The open list below was re-checked line by line against the code. Former items:

- **`structure` multi-file load unbounded — resolved** (already in code):
  `src/tools/structure_source.rs` stops at `MAX_MULTI_FILE_TOTAL_BYTES`.
- **Unbounded external reads — resolved.** `curl` carries `--max-time`/`--max-filesize`
  in `src/update.rs` and `src/update_fetch.rs` (already in code); interactive prompts
  read through `src/prompt.rs::read_prompt_line`, capped at 256 bytes with the rest of
  the line discarded (`ce50f97`). No `is_terminal` check was added, deliberately:
  `tests/menu_tests.rs` drives the menu over piped stdin, and piped input is a
  supported use.
- **Remaining silent failures — resolved.** `util::serialized_len` returns `usize::MAX` on a
  serializer error, not a partial count; the reconcile stat error propagates
  (`src/sync/reconcile/pipeline.rs`); unreadable ignore files and the RAM fallback warn
  (already in code). The five `NormalizerUnavailable` arms now go through
  `src/evidence/mod.rs::span_unavailable`, which keeps the real `SpanError` in the
  reason and warns (`621da84`).
- **Remaining false messages — resolved** (`ce50f97`). `disable`'s lock step maps only
  SQLite busy/locked codes to `DatabaseBusy` (`src/project/activation_lock.rs::lock_error`),
  reports a non-zero checkpoint as `CheckpointIncomplete` instead of a fabricated
  rusqlite payload, and the `UnsupportedVersion` message no longer promises a rebuild
  only the sync path performs.
- **Gates that pass vacuously — resolved.** `check_source_limits` propagates unreadable
  directories; `check_governance` reports an unreadable CODEOWNERS (both already in
  code); `check_performance` fails on a failed criterion cleanup and on an unreadable,
  malformed or median-less estimates file instead of dropping that bench (`d7068e1`).
- **Test gaps — partly resolved** (`850b09d`): `file_not_indexed` is pinned at both
  producers (binary file → `structure` and `finding`); the remaining non-panic
  `is_err()`-only assertions now name the error. Still open: see item 2 below.
- **R4-0 (`verdict()` doc comment ambiguous) — resolved** (uncommitted, 2026-10-18): the
  comment in `src/bin/check_source_limits/counter.rs` now states each band (0–199, 200–300,
  301–`HUMAN_APPROVED_CEILING` with a dated human exception only, above it always fail).
- **R4-1 (301–350 band untested; `"[human-approved "` duplicated) — resolved** (uncommitted,
  2026-10-18): four tests in `src/bin/check_source_limits/tests.rs` pin 301 and 350 passing with
  a human annotation, failing with an agent one, and 351 failing even with a human one. The
  prefix is one const, `approval::HUMAN_APPROVED_PREFIX`, used by both the producer
  (`approval.rs`) and the policy (`counter.rs`).
- **D-13 (52 production functions over 50 lines, 9 over cyclomatic 15) — dropped as a rule**
  (owner decision 2026-10-18): no function-length gate. Splitting tends to grow files, and the
  350-line file ceiling already bounds size; functions over cyclomatic 15 are handled case by
  case in normal review, not by a gate.

### A `mkdir` in a watched project failed the next tool call — resolved 2026-10-16 (uncommitted)

The watcher marks `Create(Folder)` dirty like any file (`src/watch/manager.rs`,
`handle_event`), and `reconcile_single_dirty_path` (`src/sync/reconcile/pipeline.rs`)
stat'ed with `std::fs::metadata` and then hashed the path. A directory failed the hash
with `Hash(Read { .. IsADirectory })`, so `drain_events` (`src/sync/manager_health.rs`)
returned an internal error and marked the watcher `Desynced` — every `mkdir` under
`serve` cost one failed call plus a full publish. The same `metadata` call followed
symlinks, so a symlink the watcher reported had its target indexed (even one outside
the project) although discovery never follows symlinks. Fix: the dirty path is
classified by `discovery::regular_file_metadata` (`symlink_metadata`; regular files
only), and a non-file path drops its stale row like a deletion or is ignored. Pinned
by `a_directory_in_the_dirty_set_does_not_fail_the_reconcile`,
`a_symlink_in_the_dirty_set_is_never_dereferenced` and
`a_file_replaced_by_a_directory_loses_its_row` (`src/sync/reconcile/reconcile_tests.rs`),
all three failing before the fix. `a_failed_incremental_reconcile_marks_the_watcher_desynced`
had used exactly this file-to-directory swap as its failure trigger; it now injects a
failure with a `RAISE(ABORT)` trigger on `files`, which the stat sweep (read-only) never hits.

### A failed `slugaudit install` left a full binary copy behind — resolved 2026-10-16 (uncommitted)

`run_install` (`src/install.rs`) staged the running binary as `.slugaudit.<pid>.<nanos>.tmp`
in the install directory and returned on the first failing step (chmod, retain, rename)
without removing it; nothing ever reclaimed the file. `apply_update` (`src/update.rs`)
cleaned up its own staged file, but carried a second copy of the same chmod → retain →
rename → stale-`slugaudit-mcp` sequence and of the cleanup helpers. Both now call
`install_place::place_staged_binary` (`src/install_place.rs`), which removes the staged file
on any failure and reports the failing step as `PlaceError { action, path, source }`
(`InstallError::Place`; `UpdateError::Stage` keeps its shape). Pinned by
`an_install_target_that_is_a_directory_reports_the_failed_step_and_cleans_up`
(`src/install_tests.rs`), which failed before the fix with the `.tmp` file listed.

### `slugaudit connect` blamed a missing home for a malformed config — resolved 2026-10-16 (uncommitted)

`write_json_server` (`src/connect_exec.rs`) mapped a config whose top level is not a JSON
object (e.g. `[]`) to `ConnectError::HomeUnavailable`, so the user was told their home
directory was unavailable. It now returns `ConnectError::RootNotAnObject { path }`
naming the file (`src/cli.rs`). `ConnectError::AgentMissing` was never constructed and
is removed (the R8-5 note in `.planning/archive/audit_2026-10-16.txt` also called
`RemoveFailed` dead; it is constructed by `connect_cli::run_remove`). The duplicated
menu-answer parsing in `run_connect_interactive`/`run_disconnect_interactive` is one
`parse_choice` (`src/connect.rs`). Pinned by six tests appended to `src/connect_tests.rs`
(`a_json_config_whose_top_level_is_not_an_object_is_refused_by_path` and five siblings
covering `SectionNotAnObject`, `UnknownAgent`, `InvalidChoice`, `RemoveFailed`, `AgentCommand`).

### Open — in priority order

1. **`check_no_duplicates` cannot pass — resolved (stale).** The gate scopes the
   commit-subject check to `origin/main..HEAD` (or `SLUGAUDIT_CHECK_COMMITS_BASE..HEAD`)
   since `4de8e9b`, so the published duplicate `style: apply rustfmt changes from CI`
   is outside its range; it passes on `main` as of 2026-10-16. No history rewrite needed.
2. **Test gaps — resolved 2026-10-16 (uncommitted).** The error-variant survey
   (enums named `*Error` under `src/`, non-bin, variant names grepped in tests) was
   re-run; every `ErrorCode` turned out tested through its wire string. Tests now name
   `SpanError::ReversedPosition`, `MigrationError::{ReadVersion, Apply}`,
   `ActivationError::{Root, CheckpointIncomplete}`, `InstallError::{NoHome, Mkdir,
   Place}` and every reachable `ConnectError`. Writing them found three bugs (the
   `mkdir`, staged-binary and connect-misreport entries above). Left untested,
   deliberately: `DiscoveryError::NotRelative` (the walker only yields paths under the
   root — defensive), `DiscoveryError::Walk` and `InstallError::CurrentExe` (no
   portable way to provoke them as root). `tests/gate_binaries.rs` now spawns every
   `check_*` binary: unknown argument → exit 2, empty `--root` → failure, and the
   source-limit verdict reaching its exit code. That last test found a fourth bug:
   `check_source_limits` passed a root with no `src/` (exit 0) where every other gate
   fails closed; it now exits 2.
3. **`path:line` citations in the docs rot on every edit — no gate covers
   them.** The docs carry 91 citations of the form `src/x.rs:NNN`. All 91 were
   in range after this session's changes, but six were pointing at the wrong
   line — four in `health.rs`, whose restructure moved them, and the runbook
   also named two health fields by paths the regroup had renamed
   (`health.watcher_health`, `health.revision_id`). An operator following §2
   would have looked for a field that no longer exists. Both are fixed, but
   nothing will catch the next occurrence. Options: a gate that at least
   range-checks every citation (cheap, and it would have caught none of these
   six — they were in range but wrong), a gate that requires each citation to
   carry a short quoted anchor so drift is visible, or dropping line numbers in
   favour of symbol names, which cannot rot. **Owner decision** — the third is
   the only one that removes the failure mode rather than detecting it.
4. **Function and file length — resolved 2026-10-17: kept as designed.** The owner
   decided the line limit stays. Its purpose is not to cap size but to force a
   conversation: a file past the floor needs a `slugaudit-line-exception` the owner
   accepts only when the extra lines are truly justified. A complexity gate was
   proposed as a replacement and declined, because it never makes anyone justify the
   code to the owner. Splits made on 2026-10-16 (`scan_step`, `check_performance`'s
   `main`, `structure_exec.rs`, `grouped_counts`) were by responsibility, not length.

## Connect surface: what is verified, and what is left

The registry no longer asserts anything nobody has run — that is what its `Evidence` tier,
`reference` version and `docs_url` are for. Current state, to finish from:

State as of 2026-10-17 (23 clients; the registry was 26 until `pearai`, `trae` and `1mcp` were
removed that day — see DECISIONS 2026-10-17):

- **18 `VerifiedByProbe`** — the CLI was installed and its `mcp` subcommands probed:
  `agy`, `amp`, `bob`, `claude`, `codex`, `copilot`, `crush`, `cursor`, `droid`, `gemini`,
  `goose`, `grok`, `hermes`, `kimi`, `opencode`, `openhands`, `pi`, `q`. `gemini` moved here
  on 2026-10-17 (resolved): gemini 0.64.0 was installed in an isolated directory and
  `slugaudit connect gemini` / `disconnect gemini` ran end to end against it under a
  throwaway `HOME`; the entry landed in `~/.gemini/settings.json` and was removed again.
- **3 `DocVerified`** — GUI applications with no CLI to run, so still documentation only:
  `windsurf`, `warp`, `claude-desktop`. **Still open**, not scheduled: confirming their config
  path and shape against the installed application (for example from its bundled sources, as
  was done for `omp`).
- **2 `VerifiedFromSource`** — `zed`, read out of Zed's own settings code: `command` is a
  string with a sibling `args` array, and the nested object slugaudit used to write is the
  legacy shape that no longer deserializes; and `omp` (resolved 2026-10-17), checked against
  omp 18.12.0's own `mcp-schema.json` (the entry `connect` writes validates against it; the
  runtime load was not exercised because it needs a model provider). omp 18.12.1 could not be
  installed that day (its `@oh-my-pi/pi-natives@18.12.1` dependency was unpublished).
- **2 clients refuse by design** with a paste-ready remedy, the version the advice came
  from and a docs link: `goose`, `crush`.

**Follow-up (small, not scheduled).** On a machine where the remaining clients are
installed, probe them the same way and upgrade `Evidence`. (`gemini` was the first, and is done —
see above.) Treat any disagreement with the recorded route as a bug in the
registry rather than as documentation drift: of the clients the two research passes
disagreed about, each pass was wrong on roughly half, and only the installed binaries
settled it.

## Phase 2 — The door: make the first minute win (items 1–4 landed; item 4 half-deferred)

**Goal:** an agent that was not going to use this tool uses it twice.

**Items 1–4 are landed and committed (2026-09-30).** `INSTRUCTIONS` was rewritten in this order —
what it is and what it adds; the first-win rule as a trigger rather than a request; the
replacement map; the operational facts (schema, payload keys, imports, structure, resources);
then the no-judgment contract once, near the end — with the creed placement the owner resolved.
`report` gained the completeness envelope plus the file-existence rows behind it (item 2), the
tool errors gained machine-readable codes (item 3), and the not-enabled state became one
actionable instruction behind every tool (item 4). Item 4's optional tool-hiding half is
deliberately not implemented (see it).

1. **`INSTRUCTIONS` rewrite — landed 2026-09-30.** Written in this order: (a) what it is
   and what it adds, in the agent's interest — budget as a consequence, not the pitch;
   (b) the first-win rule as a trigger (*about to open more than ~3 files to answer one
   structural question? query instead*); (c) the replacement map (`grep` → `query`/`leads`;
   `cat` → `SELECT content`; import tracing → `dependency_edges` CTE; notes → `finding`);
   (d) the operational facts (schema, payload keys, imports, `structure`, the
   `slugaudit://` resources); (e) the no-judgment contract **once, near the end**, at full
   strength (owner resolution below). *"Nobody is asking you to prefer SlugAudit"* and the
   *"WHEN NOT TO USE THIS"* heading are gone; two blind-spot declarations were added —
   callee-based `leads` mechanics are Rust-only (zero rows means *unchecked*, not
   *absent*), and `detail` is clipped at 200 chars and now flags it. Still owed from this
   item: moving the payload keys out of the briefing into a resource (Phase 4 item 1).
2. **`report` returns surprises, not just counts — landed 2026-09-30.**
   The completeness envelope: skipped files and why, unparsed files, binary files, files
   with no evidence, evidence caps hit. `ReportResponse` (`src/tools/report.rs:72`) now
   carries `completeness` (`src/tools/report_completeness.rs`): counts by `file_kind`
   (`indexed`/`binary`/`skipped`), `skipped_file_count`, `skipped_by_reason`, counts by
   `extraction_completeness` (so `ContentOnly` — text indexed, nothing parsed — is visible
   as itself), `index_partial`, `evidence_capped_file_count`, and
   `indexed_files_without_evidence`. Facts only: no score, severity, or ranking. The
   storage half landed with it — **every discovered file now gets a `files` row**
   (`src/sync/revision_skipped.rs`), on both the full-publish and the reconcile path, and
   `health` reports `skipped_file_count` beside its `file_count`. Decisions taken here,
   recorded so the next author does not re-litigate them:
   - **`skip_reason` is a short code** (`oversize`, `unreadable`), never the per-file human
     message: that message embeds each file's own path and size, so grouping by it would
     yield one bucket per file and `skipped_by_reason` would say nothing.
   - **Skipped rows carry a stat-derived identity**, not a content hash:
     `hash::stat_identity` = BLAKE3 over (path, byte_len, modified_unix_seconds) tagged
     `hash_algorithm = 'stat-identity'`. Reading the file to identify it would cost exactly
     what skipping avoids. Accepted consequence — a skipped file edited to the same size
     within the same mtime second is not detected — is documented in `schema.sql` and
     `ARCHITECTURE.md`, and is the stat sweep's own blind spot.
   - **A non-UTF-8-named file is the one gap that cannot be closed**: `files.path` is TEXT,
     so such a file has no row and `report` cannot count it. It stays visible in
     `PublishReport::skipped` and the server log. Do not read `index_partial` as "nothing
     at all was skipped".
   - **`PublishReport.added/modified/deleted/unchanged` now describe the whole file set**,
     skipped rows included (`added + modified + unchanged` = the existence-set size), not
     the indexed subset; `PublishReport::skipped` still names the skipped ones.
   - **Schema 4 → 5** (`files.skip_reason`, `file_kind` CHECK gains `'skipped'`); the
     mismatch path discards and rebuilds, so no migration was needed.
   - The manifest hash now includes skipped rows, so every existing project rebuilds once —
     accepted, the database is disposable.
   Consumers audited and updated in the same round, each with its own test: `report`'s
   counts, `health`'s file count, `publish_diff` (a skipped file is on disk and must not
   diff as `Deleted`), `compute_manifest_hash`, `sweep`'s stored-set comparison,
   `known_paths` (union kept and documented as still-required, not simplified),
   `leads`' `credential_files` (a credential-shaped skipped file is now reported rather
   than hidden), and the reconcile pipeline (oversize paths are no longer hashed — their
   existence row is the stat identity, so a sweep stops re-reading them).
3. **Branchable errors — landed 2026-09-30** (`fb6eea5`). `src/error_codes.rs` owns a
   closed set of six codes — `sync_budget_exhausted`, `project_not_enabled`,
   `stale_revision`, `unknown_mechanic`, `sql_too_large`, `file_not_indexed` — each a
   lowercase stable string with a doc comment and a static remedy, attached as
   `ErrorData.data = {"code": …, "remedy": …}` (`payload`/`budget_payload`). The code is
   attached where the *typed* failure still exists, never guessed from a message at the
   outermost tool: the budget routes through
   `PublishError`/`ReconcileError`/`SweepError`/`SyncError::budget_elapsed_ms` into five
   sync map sites, `PublishError::ChangedDuringSample` becomes `stale_revision`, and the
   contract errors are attached at their own site. The budget remedy names
   `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS` (with the `audit`/`lean` profile values) and says
   outright that retrying unchanged fails identically. The list is tabled in
   `.planning/ARCHITECTURE.md` (invariant 13 + the `error-codes` markers) and pinned by
   `error_codes::tests::the_documented_table_matches_the_code_set_exactly`, so a code
   cannot be added without its row. `INSTRUCTIONS` gained one sentence naming
   `ErrorData.data.code`. Note for the next author: the *"often transient — retry"*
   wording lives in `tools::context::internal_error`, which wraps database/transaction
   errors — it never wrapped the sync budget, whose message has always said "publish
   exceeded its wall-clock time budget". The remedy, not the message, is what makes the
   budget self-healing.
4. **Disabled state must not be eight failures — (a) landed 2026-09-30** (`fb6eea5`);
   (b) deliberately not implemented.** (a) `manager_orchestration::map_activation_error`
   is the single funnel behind every tool, so a not-enabled project now yields one
   `project_not_enabled` payload with the `project_control {"action":"on","path":…}`
   remedy instead of one opaque failure per tool, and `health` answers *normally* for
   such a path: `phase: "disabled"` plus the same `remedy` (`HealthPhase::Disabled`,
   `HealthResponse::remedy`; the no-`path` case reports `no_active_project` with the same
   remedy). (b) **Not implemented, and deliberately so:** hiding non-`project_control`
   tools requires `notifications/tools/list_changed`, which clients do not uniformly
   honor, so a client that ignores it would be left with **no** slugaudit tool at all —
   the exact "capability invisible" failure the compatibility constraint forbids. Since
   (a) alone turns the disabled state into one actionable instruction, the filter buys
   nothing that justifies that risk; it stays unbuilt and optional.

**DoD:** a cold agent's first two calls return something it didn't ask for and can use.

## Phase 3 — Recall at scale (landed 2026-09-30)

**Goal:** find what selective reading misses, and let the agent finish.

**Landed 2026-09-30, committed as `9ec589b`.** Items 1–9, plus the `limit: 0`
half of correctness follow-up 1. The `leads` row cap is gone: `DEFAULT_LEADS_ROWS` /
`MAX_LEADS_ROWS` are deleted and a page is cut only by the caller's `limit` and by
`max_query_response_bytes` (the same budget `query` spends), measured as the sum of each
row's serialized JSON **after** the `detail` clip, with one row always returned. Every
rows response carries `returned` / `total` / `complete`, the file denominators
`files_total` / `files_covered`, `page_file_counts`, and `limited_by`
(`none` / `limit` / `bytes`) with `row_budget_bytes` / `rows_bytes`. Each `MechanicCount`
carries `files` (distinct files — the headline denominator) and `covers`
(`rust_only` / `all_languages`). `LeadsRequest` gained `pattern` (GLOB over the path,
compiled like `structure`'s — bound, never interpolated; the predicate is parenthesized
before the glob is AND-ed in, because `credential_files` is an `OR` chain) and
`optional expected_revision`. `query` gained `expected_revision` too; a mismatch is
refused with the `stale_revision` code and a new `error_codes::stale_page_payload()`
remedy that restarts the sweep from offset 0 rather than retrying the offset. The
catalogue moved to `src/tools/leads_mechanics.rs` to stay under the 200-line production
ceiling. Tests: 17 in `tools::leads` (including distinct-file denominators on an
incomplete page, the byte budget, the pattern filter over an `OR`-chain predicate, and
the stale-pin refusal) and a new stale-pin test in `tools::query`; the pre-change
behaviour was captured by running the same probe against `HEAD` in a throwaway worktree
(100-row default, `truncated` only, the pin accepted and ignored). **The rest of the phase landed in the same commit through the parallel workstream:** items 5
and 8–9 — the three producer-less evidence kinds (`Chunk`, `RawNode`, `Metric`) are deleted and
the schema goes 5 → 6; `check_producer_coverage` is a new gate requiring a value-position
construction for every enum variant (match arms excluded, because the SQL mirrors enumerate every
variant and counting any mention would make the gate vacuous); and `files.file_class`
(`authored` / `generated` / `vendored`, orthogonal to `file_kind`, since a generated file is
still indexed) lets `report` say which files those are. The completeness envelope across
`report`/`leads`/`health` remains Phase 2's.

1. **Complete-or-labeled enumeration — landed 2026-09-30.** `leads` rows: default 100, hard cap 500
   (`MAX_LEADS_ROWS`, `src/tools/leads.rs:25`), justified in-code only by "mirroring
   `query`'s `MAX_ROWS`" — a copy, not a derivation. Replace the row cap with a **byte
   budget** (mirror `max_query_response_bytes`), raise the ceiling, default to
   everything-that-fits, and always return `returned / total / complete`.
2. **Denominators on every row response — landed 2026-09-30.** `count_all` is already complete
   (`leads_query.rs`) but `rows_sql` returns `(path, line, detail)` with no context. Add
   per-file and repo totals and the distinct-file count for the mechanic, so the next
   call is seeded.
3. **Count completeness in files, not rows — landed 2026-09-30.** At 50k hits the useful number is
   `distinct files containing ≥1` — that is a denominator an agent can exhaust.
4. **Add a path filter to `leads` — landed 2026-09-30.** `LeadsRequest` is `{path, mechanic, limit, offset}`
   today, so an agent cannot ask for *unwraps under `src/auth/**`*. Without this, big
   mechanics are unusable.
5. **Classify generated/vendored files.** `file_kind` is only `indexed|binary`. A 50k
   mechanic is usually 49,800 rows in `vendor/`, and saying so collapses the problem
   into one sentence. Pure fact, no verdict.
6. **Revision guard on paging — landed 2026-09-30.** The index has no MVCC — `files`/`evidence` are replaced
   in place, never per-revision — so pinning an older revision to page against is
   impossible; instead accept an optional `expected_revision` on paged reads and refuse a
   resumed page when it changed ("revision changed mid-sweep; restart from offset 0").
   This replaces the idea of a coverage ledger: fix the ambiguity at the source instead
   of reconstructing it afterward. (`src/tools/context.rs:79` already has this guard for
   the read path; `leads` does not.)
7. **Declare the blind spots inline — landed 2026-09-30.** `Call` evidence has exactly one producer and it is
   the Rust walker, so on a non-Rust project call-site mechanics return **zero** — and
   zero reads as *clean*. The response must say *unchecked, not absent*, the way
   `report.import_resolution` already warns that traversal is a dead end.
8. **Dead kinds.** `Chunk`, `RawNode`, `Metric` have no producer
   (`src/model/evidence.rs:28-30`; their only construction is a test fixture in
   `src/evidence/sql.rs`). Delete them, or implement the raw-tree escape hatch — but do
   not advertise kinds that cannot occur.
9. **A gate for producer coverage, so this class cannot come back.** Phase 1 found
   `ExtractionCompleteness::ContentOnly` never constructed and three evidence kinds with
   no producer; item 8 deletes them ad hoc, which fixes today's instances but not the
   class. Add the mechanical check instead: **every `EvidenceKind` variant has at least
   one producer, and every `ExtractionCompleteness` variant is reachable** — extend
   `check_docs_drift` or add a `check_schema_state` bin (registered per *Repo coupling*).
   The model specimen already exists and keeps no hand-kept copy:
   `store::connection::tests::every_evidence_kind_text_passes_the_kind_check`
   (`src/store/connection_tests.rs:464`) derives the variant list from a test-build
   `JsonSchema` of the enum — the `src/store/schema.sql` comment that names it sits above
   the `evidence.kind` CHECK (`src/store/schema.sql:80-82`) — and the Phase 1 review
   proved the shape catches a 12th variant.

**DoD:** an agent can enumerate any mechanic to completion, or be told it cannot and why.

## Phase 4 — The surfaces that make it worth keeping (landed 2026-09-30)

**All three items landed** (`8c03f9e`, `a7f327c`): the payload-key reference moved into a
`slugaudit://` resource and every tool gained an `outputSchema`, a `title` and annotations, with
`check_mcp_surface` refusing any drift unless `--record` is passed; blast radius returned as
inbound/outbound edge arithmetic in `report`, with the traversal shipped as a recipe resource
rather than a ninth tool; and `leads` gained `mode: compact`, which returns the whole mechanical
surface as `mechanic, file, line` triples in one response. `report` and `leads(path)` stayed
zero-decision: `required` is still just `["path"]`.

1. **Payload-shape resource + `outputSchema` on all 8 tools.** Per-kind payload keys
   already exist as prose inside `INSTRUCTIONS` — they cost context every session and
   teach the model only if read. Move them to a `slugaudit://` resource, and add
   `outputSchema`, `title`, and MCP `annotations` (`destructiveHint` on
   `project_control off`, which purges the DB) so a client can learn the contract without
   a call. Pair with a surface-snapshot gate so the schema cannot drift (see the
   interface-contract item in the backlog, which is the same work).
2. **Blast radius as a fact.** The old brief's value was changed-files + blast radius +
   embedded source. Inbound/outbound edge counts are arithmetic (`dependency_edges`), not
   judgment. Return them; ship the traversal recipe as a resource rather than a ninth
   tool.
3. **Zero-decision entry points.** `report` and `leads(path)` need no schema knowledge;
   keep them that way, and add a compact `mechanic, path, line` enumeration mode so the
   whole mechanical surface can be seen in one complete response.

## Phase 5 — Close the loop (item 1 landed 2026-09-30; item 3 conditional, not scheduled)

**Item 1 landed** (`d9cb506`): `finding` dedupes a restated conclusion inside the session by
(path, source_hash, line range, category, title), updating the row in place and keeping its id,
while a restatement after the file changed becomes a new finding and the old row stays stale.
**Item 2 is an instruction not to act**, honoured: session scoping is untouched.
**Item 3 is conditional and stays unscheduled** — it is worth building only if Phase 0's sequence
log shows agents losing their place, and there is no adoption data from real audits yet; Phase 3's
revision guard already covers most of the value.

1. **Finding dedupe.** `finding` unconditionally `INSERT`s (`src/tools/finding.rs:104`),
   so re-recording the same conclusion duplicates rows. Python deduped by
   `identity_hash`; restore that inside the session.
2. **Do not restore cross-session finding durability.** Session scoping is deliberate and
   correct: the DB must never be the sole holder of a fact.
3. **Ledger — only if measurement says so.** A per-session record of which rows were
   *returned* (memory-only, never in the DB, worded as "returned", never "examined") is
   worth building only if Phase 0's sequence log shows agents losing their place. Phase
   3's revision guard covers most of the value at a fraction of the fragility.

---

## Anti-goals

- Restoring `risk_score`, severity enums, ranked `risk_leads`, or any ranked brief.
  The old rave came bundled with ranking; the ranking is what makes output arguable, and
  arguable output is what sends an agent back to grep.
- Any vanity copy — testimonials, self-praise, or a rave placed in the tool's own docs.
  Praise from inside the documentation is worth zero and is gameable.
- Optimizing for sentiment. A rave is cheap and sycophantic; a call sequence is not.
- Turning a row cap into a bigger row cap. The unit is bytes.

## Acceptance (the only metric that counts)

Over the next N real audits, from the Phase 0 sequence log:

- agents make **chains** (`report`/`leads` → `query`/`structure` → `finding`), not 2–3
  calls then grep;
- sessions where the tool is used report **extra findings** that grep-and-read missed;
- sessions finish faster *with* the extra results.

## Kill criterion

If, after Phases 1–4, agents still make a few calls and return to grep, and sessions
using the tool stop producing extra findings, then the hypothesis **"agents want an index
of their codebase"** is false. Say so and stop — do not re-brand, do not add surfaces,
do not go looking for a different audience. The behavior is the product.

## Decisions only the owner can make

**Resolved.**

- **Placement of the no-judgment contract in `INSTRUCTIONS`** — resolved 2026-09-30.
  The contract stays **equally strong**, but it moves **after** the statement of value
  and the first-win rule, and is stated **once, near the end**. It is never the opener
  and never softened into a disclaimer. This unblocks Phase 2 item 1.
- **Audience** — the tool is built for AI agents, not humans ("it's not a human tool,
  it's a tool built to be used by you"). The only human-facing text that survives is
  install/connect/operator material (`README.md`, `docs/CLIENTS.md`,
  `docs/TROUBLESHOOTING.md`). Nothing in a tool response is written for a human reader.

**Still open.**

- The thresholds in Acceptance: how many sessions N, and what counts as "extra findings".
- Whether any human-facing *positioning* beyond install/connect/operators returns. Today's
  README-shaped positioning is stronger than the artifact; it is the first thing to
  weaken if the audience is agents.
- The credential-content policy in the backlog (store hash/metadata and omit content, or
  store full text): a product decision, not a code decision.
- **Governance contacts.** `CODE_OF_CONDUCT.md` has no private enforcement address, so the
  canonical placeholder is filled with this repository's **public** issue tracker — disclosed in
  the file, but public by construction, and a conduct report should not be. `SECURITY.md` uses
  GitHub's private security advisories, which is the standard channel, but the repository
  publishes no security address either. Decide whether to publish a private contact for conduct
  and security reports (the file says plainly that none is published today).
- **`1mcp` is a gateway, not a client.** It aggregates other MCP servers and is fronted by
  real clients (`1mcp serve`, `1mcp proxy`), so `1mcp mcp add slugaudit …` registers
  slugaudit as a *backend of 1mcp* rather than connecting slugaudit to an agent. Keep the
  target with that framing, or drop it from the list.

---

## Production-readiness backlog (hygiene; adds no product features)

Folded from `PROD_READINESS_PLAN.md`. Items already landed are dropped; the evidence
phase of that plan is now Phases 1–3 above (its `known_paths` fix and its `ContentOnly`
item are done, its report-completeness and dead-kind items live in Phase 2 item 2 and
Phase 3 item 8). **Landed 2026-09-30:** store safety's directory/sidecar permissions
(`2b64cc3`), the governance shell's five files (`ec10812`) plus its `check_governance` gate,
the exposure documentation (`SECURITY.md`), the install/update rollback (`5a59e69`), the
adversarial `query` tests (`210ecaa`), the version-consistency gate, the credential predicate
(`47f733d`), auto-ignore on enable (`b451b02`) and the supply-chain pins/scans (`fd9826e`).
The rest is open.

**Store safety and secrets.**

- **Landed 2026-09-30** (`2b64cc3`). `.planning/slugaudit/` was created with plain
  `create_dir_all` — whatever the process umask allowed. It is now created `0700` and
  tightened when an earlier run left it wider, and the WAL/SHM sidecars (created by SQLite,
  not by this crate) are pinned by a test: SQLite derives their mode from the database
  file's, which is `0600` (`PRIVATE_MODE`, `reject_insecure_permissions`,
  `src/store/connection.rs`), so they come out owner-only in practice.
- **Auto-ignore on enable — landed 2026-09-30** (`b451b02`). Nothing in `src/` wrote a
  `.gitignore` entry, so a customer's first `git status` showed the database blob and its
  sidecars untracked. `project_control {"action":"on"}` now appends `.planning/slugaudit/`
  unless an equivalent rule is present, append-only (prior bytes preserved, trailing-newline and
  CRLF conventions respected), built from the same constant the index-exclusion rules use. A
  failed write is reported as `not_written`, never fatal: the activation directory is what makes
  a project enabled. `off` never touches the file.
- **Credential-content decision** (owner decision above). Files matching
  `CREDENTIAL_PATH_PREDICATE` are *identified* for `report`/`leads`, but their full text
  is still stored in `files.content`. Store hash/metadata, omit content, and mark the
  file so the agent knows the text is deliberately absent and must be read from disk.
- **The predicate itself is too narrow in places and too broad in another — landed
  2026-09-30** (`47f733d`). It now covers the root-level SSH keys, `.netrc`, `.npmrc`,
  `.git-credentials`, `id_ecdsa`, `*.p8`, `*.jks`/`*.keystore`, `*.tfstate` and
  `service-account*.json`, and stops counting `id_rsa.pub`. Two SQL traps were fixed on the way
  and both are pinned by cases: `_` is a single-character `LIKE` wildcard, so `%id_rsa%` also
  matched `id-rsa`/`idXrsa` (now escaped with `ESCAPE`), and `AND` binds tighter than `OR`, so
  the `.pub` guard without its own parentheses applied to the last term only. The table-driven
  test (49 pairs) evaluates the shared fragment **through SQLite**, so it cannot pass while
  `report` and `leads` disagree. The credential-*content* policy remains an owner decision.
- **Document the exposure — landed 2026-09-30** (`ec10812`): `SECURITY.md` states the DB
  location, its permissions (and the `0700`/`0600` split), that it holds full source text,
  and that it is disposable.

**Governance shell — landed 2026-09-30** (`ec10812`). All five files exist:
`.github/CODEOWNERS` (owner handle grounded in `glama.json`, `LICENSE` and the remote),
`CONTRIBUTING.md` (pointer-first; it states the DCO requirement and that it is not yet
machine-enforced), `SECURITY.md` (threat model, permissions, disclosure via GitHub
advisories), `CODE_OF_CONDUCT.md` (Contributor Covenant v2.1, fetched and diffed; its
contact placeholder is disclosed as the public issue tracker — see the owner decision
above) and `CHANGELOG.md` (newest entry `1.0.10`, dated from the real tag).

- `check_governance` — **landed 2026-09-30** (`ca6c569`): fails when a required file is
  missing or when a literal `CODEOWNERS` pattern names a path that does not exist (globs and
  negations are skipped, not silently passed).

**Release, versioning, supply chain.**

- **`check_version_consistency` gate — landed 2026-09-30** (`ca6c569`). The version is
  hand-duplicated (`Cargo.toml` = `lhm.plugin.json` = `1.0.10` = the newest `CHANGELOG.md`
  heading) and nothing tied them together. The gate asserts all of them agree and that
  `parse::PACK_VERSION` matches an *exact* `tree-sitter-language-pack` pin; it folds in the
  unit test that asserted half of that (the gate reads the manifest at a chosen root, so it
  also runs against a fixture). This covers the version half of the CHANGELOG-discipline
  item below; release-please is still open.
- **CHANGELOG discipline** — release-please, or a gate requiring an entry for the tagged
  version.
- **Rollback — landed 2026-09-30** (`5a59e69`). `slugaudit update`, `slugaudit install` and
  both `install.sh` paths now copy the binary they displace to `slugaudit.prev` **before** the
  replacement rename, so atomicity is preserved and reverting is one `cp` (the copy keeps the
  mode bits). It fails closed: a retention copy that silently does not happen would leave the
  operator believing a revert is possible. Documented in `docs/TROUBLESHOOTING.md` §7.
- **Reproducibility** (`--remap-path-prefix` + a CI job that rebuilds a tag and compares
  hashes, best-effort and documented as such); **SBOM** (CycloneDX, attached to the
  release); **provenance/attestation** (`actions/attest-build-provenance`; note explicitly
  that `SHA256SUMS` from the same origin as the artifact is integrity, not authenticity);
  **third-party notices** generated from the dependency graph; **PR-time supply chain**
  (dependency review, CodeQL or OSV-Scanner, secret scanning with `.planning/slugaudit/`
  excluded).
- **Every action is unpinned, there is no update bot, and nothing advisory runs on a
  schedule** (verified 2026-09-30). All seven third-party actions in `.github/workflows/`
  float on a mutable tag or branch — `actions/checkout@v4`, `Swatinem/rust-cache@v2`,
  `EmbarkStudios/cargo-deny-action@v2`, `rustsec/audit-check@v2.0.0`,
  `taiki-e/install-action@v2`, `softprops/action-gh-release@v2`, and
  **`dtolnay/rust-toolchain@master`, which tracks a branch**; there is no
  `.github/dependabot.yml` or `renovate.json`; and `schedule:` appears in neither workflow,
  so a RustSec advisory published against an unchanged lockfile is invisible until the next
  push. What *does* run today is `cargo-deny`, `rustsec/audit-check` and `cargo-mutants`
  (four files, Linux-only) — no CodeQL, OSV or secret scanning. **Landed 2026-09-30**
  (`fd9826e`): every action is pinned to a full commit SHA — the *peeled* commit for annotated
  tags, since a tag object is not a commit — with the version in a comment;
  `.github/dependabot.yml` covers cargo and GitHub Actions weekly and records that the
  exact-pinned parser pack has no other bump path and that a bump must move
  `src/parse/PACK_VERSION`; `.github/workflows/advisories.yml` runs the advisory check on a cron
  against the same `deny.toml`, so the parser pack's indirect exposure surfaces without a push;
  and `.github/workflows/supply-chain.yml` runs dependency review, OSV scanning and secret
  scanning on pull requests, with `.planning/slugaudit/` excluded from secret scanning so a
  developer's local index is never uploaded. The `rustsec/audit-check` step was removed: it read
  the same advisory database as the cargo-deny step beside it, and its `checks: write`
  permission went with it.
- **`check_performance` is a gate that CI does not run — resolved 2026-09-30** (`fd9826e`).
  `Cargo.toml` registers it and the docs listed it, `quality.yml` contained zero references to
  it, and the workflow comment referred to a performance gate "below" that was never added, so
  the allowance was documentation rather than enforcement. Resolved by stating it as
  **local-only** in both workflows and the three contributor docs, with the reason (criterion
  timing on shared runners) and the baseline's recorded machine.
- **Third-party attribution is a release step, not an idea.** A release ships a
  third-party attribution list covering the dependency graph, sourced from a per-crate
  licence/FFI inventory, and a new dependency that introduces transitive unsafe is named
  there (the obligation `AGENTS.md` records). The `LICENSE` Notices clause
  (`LICENSE:71-79`) is the model: whoever receives a copy gets the terms and the
  `Required Notice:` lines. The per-crate inventory this used to be read from —
  `.planning/archive/DEPENDENCIES.md` — was deleted in the 2026-09-30 reset, so the list
  must be **regenerated from the dependency graph** (`cargo-about`/`cargo-deny` output)
  rather than copied from a file that no longer exists. **Landed 2026-10-03** (`7fe3e04`):
  the release workflow generates the notices with `cargo-about` from that graph, using
  `docs/about.toml` (whose accepted-licence list mirrors `deny.toml`, so the notices and the
  licence gate cannot disagree) and `docs/third-party-notices.hbs`, and attaches them.
- **Done-condition — met 2026-10-03, except the dry run.** The release produces the binary +
  `SHA256SUMS` + a CycloneDX SBOM + the third-party notices + a build attestation, and
  `docs/RELEASING.md` documents **how a user verifies each** — including that a same-origin
  `SHA256SUMS` is integrity, not authenticity, which is what the attestation adds. The
  remaining part is inherent: the workflow has never executed, because that needs a tag push
  and publication is the owner's call. Two things were found and fixed while building it: a
  manual dispatch used to build the dispatched branch and tag the result with the operator's
  input (so the published binary need not have matched its tag, and would now have been
  attested as if it did), and the reproducibility comparison's first version mutated the bytes
  it was comparing. The reproducibility limit is measured rather than assumed —
  `.rodata`/`.strtab`/`.note.gnu.build-id` differ because `ahash`'s `compile-time-rng` embeds
  a random constant at compile time — and the job warns on exactly that instead of claiming a
  bit-for-bit rebuild.

**Untrusted-input rigor.** `structure` executes agent-supplied Tree-sitter queries;
`query` executes agent SQL under a read-only connection, a subquery wrapper, and an
authorizer hook. These are the most interesting surfaces in the threat model, and both now
have fuzz targets (`aa594f1`). Mutation testing covers four files, Linux-only — deliberately,
and the reason below is measured rather than asserted.

- A `fuzz/` crate (cargo-fuzz) with three targets: Tree-sitter query strings fed to
  `structure`; SQL fed to `query`, including authorizer-bypass attempts; JSON-RPC frame
  parsing. Seed from `examples/queries/*` and `src/store/schema.sql`. Nightly/scheduled
  workflow, not per-PR. **Landed 2026-10-03** (`aa594f1`), with one honest correction: there
  is no frame parser here to fuzz — `src/main.rs` hands stdio to `rmcp`, which owns framing —
  so the third target fuzzes the `params.arguments` decode this repo *does* own, and says so
  in its header rather than implying it covers a dependency's parser. Bounded runs were clean
  (219 / 316 / ~203k execs per second; peak RSS well inside the 2048 MB limit) and produced
  one finding: an **upstream** leak in tree-sitter 0.26.13 that nothing here can free,
  suppressed precisely in `fuzz/lsan-suppressions.txt`. The leak, the evidence that it is not
  this crate's, and the symbolizer trap that makes the suppression silently ineffective are
  recorded in `DEVIATIONS.md`.
- Adversarial authorizer tests as permanent unit tests (`ATTACH`, `PRAGMA`,
  `load_extension`, writes through CTEs/subqueries, comment/`;` splicing, multiple
  statements) asserting rejection, not just "no rows". **Landed 2026-09-30** (`210ecaa`):
  six tests over ~46 payloads in `src/tools/query_safety_tests.rs`, plus three
  weakened-guard experiments proving they are not vacuous (with the wrapper and the
  authorizer both removed, `ATTACH` returns `Ok([])` — the silent false pass — and every
  query test but `store::connection`'s still passes, which is where the authorizer is
  actually pinned).
- Widen mutation testing or document why the narrower scope is intentional. **Documented
  2026-09-30** (`f153158`), with the measurement rather than an assertion: the four files
  hold 23 of the crate's 2323 mutants, while `src/sync/**` holds 310 and `src/tools/**` 484,
  and cargo-mutants rebuilds and re-runs the whole suite per mutant.
- A synthetic large-repo soak test that stays inside `max_sync_wall_clock` (nothing
  exercises a 10k-file import except the budget that would abort it). **Landed 2026-09-30**
  (`f153158`): `tests/large_repo_soak.rs` indexes 10,000 files through the real sync path in
  18.8 s against a 1200 s budget, asserting the exact path set, not merely "no error". The
  10k case is `#[ignore]`d and a 64-file companion guards the same assertions in the default
  suite in 0.13 s.
- **Done-condition — met 2026-10-03.** The fuzz targets build and run clean for a bounded
  time on a schedule rather than per-PR; the adversarial authorizer tests are green and assert
  rejection rather than only "no rows"; and the large-repo soak test completes well inside
  `max_sync_wall_clock` (18.8 s against 1200 s).

**Shipped/dev artifact split.** `Cargo.toml` declares six bins — one user-facing, five
development gates — and the repo's own `.planning/` development docs sit beside the
runtime namespace.

- Convert to a workspace: `crates/slugaudit` (product) and `crates/slugaudit-gates` (the
  six `check_*` bins); keep `Cargo.lock`, `deny.toml`, `rust-toolchain.toml` and any
  `.cargo/config` at the root; `release.yml` builds only the product crate. This renames
  every path — see *Sequencing*.
- `scripts/fast-check.sh` was deleted on 2026-09-30 (it disagreed with the real gate on
  both thresholds and annotation parsing, so a green fast-check proved nothing). If a fast
  local check is wanted, fold it into the gates crate with thresholds and annotation
  parsing **identical** to `check_source_limits`.

**Interface contract and observability (residual).** The parts that Phase 0 and Phase 4
do not already cover:

- **Pin the protocol version.** `src/server.rs:346` advertises `ProtocolVersion::LATEST`,
  which moves whenever `rmcp` is upgraded. Pin explicitly (`V_2025_06_18` matches the
  product's documented compatibility claim) and, if clients support it, advertise the
  supported range via discovery (`V_2024_11_05` … `V_2026_07_28`).
- **Call correlation** — Phase 0 item 2 does this; the two are the same work.
- **MSRV — an owner decision, not an implementation task.** Either declare
  `rust-version`, or record the deliberate policy that only the pinned toolchain is
  supported. `Cargo.toml` has no `rust-version` key today and `AGENTS.md` states there is
  no declared MSRV and none should be invented, so that is what holds until the owner
  decides.
- **Ship the runbook — landed 2026-09-30.** `health`'s full-publish runbook used to
  point at `.planning/PERFORMANCE.md`, a development-only file. It now points at
  `docs/TROUBLESHOOTING.md` (`src/tools/health.rs:98`, `src/sync/manager.rs:79`), and
  the development-only file is gone. Keep the pointer inside `docs/` — `.planning/` is
  a development record and would be customer-owned in a real project.
- **Done-condition.** The surface-snapshot gate (Phase 4 item 1) is green on the committed
  `tools/list` + `resources/list` JSON, and the error-code list (Phase 2 item 3) is
  documented and pinned by a test.

**Config precedence.** `ProjectConfig::apply_to_limits` rebinds the whole struct when a
project `profile` is set (`src/project/config.rs:65`), discarding every `SLUGAUDIT_*` env
override already applied, and unparseable env values silently fall back to defaults. Make
the precedence explicit and layer rather than replace; warn on unparseable input.

---

**Deliberately out of scope.** Google requirements that do not transfer: SLOs, on-call
rotations, canary/gradual rollout, staging environments, capacity planning, incident
process, a metrics pipeline. A local stdio tool has no fleet to roll out to; the
transferable analogue is diagnosability (structured stderr logs,
`SLUGAUDIT_LOG_FORMAT=json`, the `health` snapshot), which Phase 0 completes. Separately
tracked product design — not hygiene — is: a span-addressed read tool
(`read_lines`/`read_span`), findings durability and export, the content-chunking decision,
and `leads`-vs-resources consolidation. Those change what the product does.

---

## Repo coupling (violating these fails CI)

- Every phase ends with the full gate suite green: `cargo fmt --check`, `check`,
  `clippy -D warnings`, `test`, and the six `check_*` bins (`check_source_limits`,
  `check_no_duplicates`, `check_docs_drift`, `check_producer_coverage`, `check_coverage`,
  `check_performance`).
- **Anything added, renamed, or removed under `src/` requires updating the fenced module
  map in `.planning/ARCHITECTURE.md`**, and adding or removing a `src/**/*_tests.rs` file
  requires updating the "*N* `*_tests.rs` files" sentence in that same file.
  `check_docs_drift` fails otherwise.
- **`check_docs_drift` also checks the canonical doc set**: the documentation files that
  exist on disk must be exactly the canonical set this reset defines. Adding or deleting a
  doc means updating the gate's expectation in the same commit.
- Test function names and commit subjects must be **globally unique**
  (`check_no_duplicates`).
- `check_source_limits`: 200 code lines per production file; 200–300 needs an annotation;
  `*_tests.rs` caps at 500.
- New gates are pure-Rust cargo bins under `src/bin/` (repo idiom: no shell, no Python).
  Each new bin must be registered in `Cargo.toml`, added to
  `.github/workflows/quality.yml`, and listed in the `ARCHITECTURE.md` module map.
- No new dependency without clearing `cargo deny`.
- Schema changes bump `CURRENT_SCHEMA_VERSION` (`src/store/migrations.rs:9`); the mismatch
  path discards and rebuilds, so no migration is needed.
- `check_performance` consumes `.planning/archive/perf_baseline.json` — never delete it.

## Sequencing

```
Phase 0 ─ Phase 2 ─┐
Phase 3 ───────────┼─ (independent) ─→ workspace split ─→ surface snapshot
Phase 4 ───────────┤
Phase 5 ───────────┘
```

Phases 2–5 are independent of each other in the code. **The workspace split goes last**:
it renames every path, so the plan's `src/` work must land first, and the interface
snapshot must be taken after both, since it pins tool names and schemas this plan changes.
The split must land as one commit that also updates the `ARCHITECTURE.md` module map, the
`*_tests.rs` count sentence, `include_str!` paths in `src/resources.rs`,
`check_docs_drift`'s expectations, and `check_source_limits` counts.

---

## Review findings and their disposition (Phase 1, 2026-09-30)

Phase 1 landed with an independent peer review of the diff. Recorded so the next author
does not rediscover them.

**Fixed in the same round:**

- **Duplicate whole-tree sweep** (introduced by Phase 1 item 1, caught by two reviewers
  independently): `sync_by_health`'s `Healthy` branch calls `reconcile()` — which gained a
  sweep for `on_disk` — and then `sweep_and_reconcile()` *outside* the `if`, so the
  edit-then-query hot path walked and stat'd the tree twice per call and the first walk
  drew down the shared deadline (a previously-succeeding reconcile could abort with
  `TimeBudgetExceeded` and flip the watcher `Desynced`). Now: one walk per Healthy call,
  one deadline for walk + event reconcile + apply, and the single-walk property is
  test-pinned by a `cfg(test)` walk counter in `src/sync/sweep.rs`.
- **`Full` computed before evidence caps** — a file whose evidence was discarded by
  `apply_evidence_limits` was stored `Full` beside a "records discarded" marker.
  `apply_evidence_limits` now reports whether it discarded anything and `to_file_record`
  downgrades `Full` → `Partial`.
- **Completeness was invisible and unparsed files read as bare parser failures** —
  `report`'s failure rows now carry `extraction_completeness` + `parser_availability`, so
  *content indexed, not parsed* is distinguishable from *parse attempted, failed*.
- **Silent cap in `report`** — `parser_failure_files_total` added, so the capped list
  states whether it is complete. Same treatment still owed to: `structure` match cap,
  per-file evidence caps, and the `leads` detail clip's *count* (Phase 2 item 2 / Phase 3).
- **Guard that could not fail** — the `evidence.kind` CHECK test kept its own copy of the
  variant list, so a 12th variant would pass the test and fail at insert time. The list is
  now derived from a test-build `JsonSchema` of `EvidenceKind`, so there is no copy to
  forget; proven by injecting a 12th variant (compile error in `kind_text`, then a test
  failure naming the CHECK).
- **Stale literal** — `sync::manager::tests` asserted `PRAGMA user_version == 3`; now
  asserts against `store::CURRENT_SCHEMA_VERSION` (test-only re-export).

**Deferred, with reasons:**

- **Path-separator normalization for `sweep.on_disk`** — the sweep inserts raw
  `Path::to_str()` while discovery normalizes to `/`. On Windows the reconcile-side half of
  the `known_paths` fix would silently do nothing. Deferred because releases are Linux
  x86_64 only (`release.yml`) and the pre-existing sweep comparison shares the same
  mismatch; fix both together if Windows ever becomes supported.
- **Project config was ignored on the reconcile path — resolved 2026-10-14.**
  `ReconcileOptions::for_sync` now receives the project root and applies `ProjectConfig`
  to the process limits exactly as full publish does. A watcher-backed behavioral test
  grows an indexed file beyond the configured limit and verifies reconciliation replaces
  it with an honest skipped row, so an edit can no longer be accepted or skipped solely
  according to whether watcher reconciliation or full publication happened to run. The older
  skip-aware reconcile test remains at the `sweep_tree`/`apply_sweep_report` seam because
  it exercises that boundary directly.
- **`extraction_completeness` is still read by no tool except `report`'s failure rows** —
  the completeness envelope across `report`/`leads`/`health` remains Phase 2.
