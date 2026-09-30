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

**Committed (2026-09-30).** One commit per workstream, each leaving the full gate suite green:
`ec6a464` (the deviations log), `ecc25e7` (Phase 1), `9c0f77f` (the connect registry — its
`ARCHITECTURE.md` rewrite lands there rather than with the documentation reset, because the module
map names the new connect modules and the `*_tests.rs` count sentence is gate-checked against the
new test file), `dca4910` (the documentation reset), `5302bf0` (Phase 0's instrument), `9085f6c`
(the plan/deviations bookkeeping), `b9bfb39` (Phase 2's index completeness — `fmt`, `clippy
-D warnings`, 565 lib tests and the three cheap gates verified green on it), `fb6eea5` (Phase 2's
error surface), `78a245f` (the plan status reconciliation), `9ec589b` (Phase 3). **Not pushed** —
the branch has no remote counterpart and publication is the owner's call.

**Not started:** Phases 4–5; the *Correctness follow-ups* list; the *Connect surface* follow-up;
and every item in the *Production-readiness backlog*.

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

1. Record the gate suite output so later regressions are attributable. There is no
   committed baseline file for the gate run; `check_performance` has its own
   (`.planning/archive/perf_baseline.json`, regenerated with `--record`).
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

Found while auditing the surfaces, verified in the code, and **not yet fixed**. Each is
independently small, and none is covered by the phases below.

1. **`leads` with `limit: 0` can page forever — resolved 2026-09-30** (the row cap is gone; `limit` is clamped to `>= 1`, and the response reports `limited_by`/`complete`, so a resumed page always advances). `limit` is only floored at the maximum
   (`unwrap_or(100).min(500)`), so `LIMIT 0` returns an empty page while `truncated` stays
   `true` and `next_offset` equals the offset it was given — a caller following the
   documented paging contract loops on the same page forever. Clamp to `>= 1`, or reject
   it as a contract error. (`src/tools/leads.rs`, `src/tools/leads_query.rs`.)
2. **`query.offset` is unbounded.** It is a `usize` interpolated straight into
   `OFFSET {offset}`, so a value above `i64::MAX` is accepted and silently mis-pages
   instead of returning a contract error. Bound it the way `limit` is bounded.
   (`src/tools/query.rs`.)
3. **`char_column` can panic on a non-boundary offset.** It slices `source[..byte_offset]`
   on a doc-comment-only precondition, so a Tree-sitter node boundary landing mid-codepoint
   panics inside `structure` on real project content — a tool call that dies mid-audit.
   (`src/model/span.rs`.)
4. **Tests execute real client binaries.** `AgentDef::is_connected` falls through to
   `which(cli)` + `cli_check_connected(cli)`, so a connect test that passes a temporary
   `home` still runs whichever client CLI happens to be installed: `codex` rewrote the
   user's `~/.codex/config.toml` during a test run on 2026-09-30 (content intact, no
   slugaudit entry). Route those tests through the existing fake-CLI harness so the suite
   touches no real client and does not depend on what is installed.

## Connect surface: what is verified, and what is left

The registry no longer asserts anything nobody has run — that is what its `Evidence` tier,
`reference` version and `docs_url` are for. Current state, to finish from:

- **18 `VerifiedByProbe`** — the CLI was installed and its `mcp` subcommands probed:
  `agy`, `amp`, `bob`, `claude`, `codex`, `copilot`, `crush`, `cursor`, `droid`, `goose`,
  `grok`, `hermes`, `kimi`, `opencode`, `openhands`, `pi`, `q`, `1mcp`.
- **6 `DocVerified`** — route taken from vendor documentation, never run here: `gemini`,
  `omp`, `trae`, `warp`, `windsurf`, `claude-desktop`.
- **1 `VerifiedFromSource`** — `zed`, read out of Zed's own settings code: `command` is a
  string with a sibling `args` array, and the nested object slugaudit used to write is the
  legacy shape that no longer deserializes.
- **1 `Unverified`** — `pearai`; the two research passes disagreed about its mechanism, so
  `connect` refuses rather than guessing.
- **4 clients refuse by design** with a paste-ready remedy, the version the advice came
  from and a docs link: `goose`, `crush`, `pearai`, `trae`.

**Follow-up (small, not scheduled).** On a machine where the remaining clients are
installed, probe them the same way and upgrade `Evidence`; `gemini` first, since it is the
one `DocVerified` client whose *argv* correctness matters (the `-s user` scope) and whose
binary was unavailable here. Treat any disagreement with the recorded route as a bug in the
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
3. **Branchable errors — landed 2026-09-30 (uncommitted).** `src/error_codes.rs` owns a
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
4. **Disabled state must not be eight failures — (a) landed 2026-09-30 (uncommitted);
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

## Phase 4 — The surfaces that make it worth keeping (not started)

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

## Phase 5 — Close the loop (not started)

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
- **`1mcp` is a gateway, not a client.** It aggregates other MCP servers and is fronted by
  real clients (`1mcp serve`, `1mcp proxy`), so `1mcp mcp add slugaudit …` registers
  slugaudit as a *backend of 1mcp* rather than connecting slugaudit to an agent. Keep the
  target with that framing, or drop it from the list.

---

## Production-readiness backlog (hygiene; adds no product features)

Folded from `PROD_READINESS_PLAN.md`. Items already landed are dropped; the evidence
phase of that plan is now Phases 1–3 above (its `known_paths` fix and its `ContentOnly`
item are done, its report-completeness and dead-kind items live in Phase 2 item 2 and
Phase 3 item 8). Nothing here has been started.

**Store safety and secrets.**

- `.planning/slugaudit/` is created with plain `create_dir_all`
  (`src/project/database_path.rs:25`) — no explicit `0700`. The database *file* is already
  hardened: created `0600` atomically and refused if broader (`PRIVATE_MODE`,
  `reject_insecure_permissions`, `src/store/connection.rs:11,157,181`; test
  `store::connection::tests`). Extend the same explicitness to the directory and to the
  WAL/SHM sidecars, and test-pin the mode bits.
- **Auto-ignore on enable.** Nothing in `src/` writes a `.gitignore` entry (the code only
  reads ignore files), so a customer's first `git status` shows an untracked binary blob
  plus sidecars. On `project_control {"action":"on"}`, append `.planning/slugaudit/` to
  the project's `.gitignore` when absent and report that in the response
  (`src/project/activation.rs`, `src/tools/project_control.rs`).
- **Credential-content decision** (owner decision above). Files matching
  `CREDENTIAL_PATH_PREDICATE` are *identified* for `report`/`leads`, but their full text
  is still stored in `files.content`. Store hash/metadata, omit content, and mark the
  file so the agent knows the text is deliberately absent and must be read from disk.
- **The predicate itself is too narrow in places and too broad in another.** It matches
  `.env`, `.env.*`, `.pem`, `.key`, `.p12`, `.pfx`, `%/id_rsa%`, `%/id_ed25519%`
  (`src/tools/mod.rs`), so it **misses** root-level `id_rsa`/`id_ed25519` (the patterns
  require a leading slash), `.netrc`, `.npmrc`, `.git-credentials`, `id_ecdsa`, `*.p8`,
  `*.jks`/`*.keystore`, `*.tfstate` and `service-account*.json`, while flagging
  `id_rsa.pub` — a public key — as a credential. Widen and tighten it with a
  table-driven test **before** relying on it for the content decision above, so the
  "which files are secret" answer and the "whose content do we withhold" answer cannot
  disagree.
- **Document the exposure**: DB location, permissions, that it holds full source text,
  and that it is disposable (belongs with the governance docs below).

**Governance shell.** All absent today (`.github/` holds only the two workflows; no
`CODEOWNERS`, `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`).

- `OWNERS` / `.github/CODEOWNERS` mapping at least `/src/sync/`, `/src/store/`,
  `/src/tools/`, `/.planning/ARCHITECTURE.md`, `/.github/`, `/AGENTS.md`.
- `CONTRIBUTING.md` — build/test/gate commands, house style, branch and commit rules
  (unique subjects), DCO sign-off, where plans live.
- `SECURITY.md` — threat model (lazy/unsupervised agent, **not** a hostile attacker),
  what the tool can read, DB location and permissions, disclosure address, supported
  versions.
- `CODE_OF_CONDUCT.md` — Contributor Covenant verbatim; `CHANGELOG.md` stub.
- `check_governance` — a pure-Rust bin that fails when a required file is missing or when
  `CODEOWNERS` references a path that does not exist (same spirit as `check_docs_drift`).

**Release, versioning, supply chain.**

- **`check_version_consistency` gate.** The version is hand-duplicated
  (`Cargo.toml:3` = `lhm.plugin.json:5` = `1.0.10`) with nothing tying them together.
  Assert equality across manifests, that the newest `CHANGELOG.md` entry matches, and that
  `parse::PACK_VERSION` matches its exact manifest pin (currently a unit test — fold it
  in).
- **CHANGELOG discipline** — release-please, or a gate requiring an entry for the tagged
  version.
- **Rollback.** `slugaudit update` replaces the binary atomically but keeps no previous
  copy. Retain `slugaudit.prev` and document a revert path (`src/update.rs`,
  `src/install.rs`, `install.sh`).
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
  (four files, Linux-only) — no CodeQL, OSV or secret scanning. Pin every action to a full
  commit SHA and let the bot bump it; add a dependency-update bot; move advisories onto a
  cron. The dependency with the most to gain is the exact-pinned parser pack
  (`tree-sitter-language-pack = "=1.14.3"`, 371 bundled grammars), which has no automated
  bump path at all.
- **`check_performance` is a gate that CI does not run.** It is registered in `Cargo.toml`,
  has a bin under `src/bin/`, and is listed among the gates, but `quality.yml` contains zero
  references to it (`grep -c check_performance .github/workflows/quality.yml` → `0`), and
  the workflow comment refers to a performance gate "below" that was never added. So the
  perf-regression allowance is documentation rather than enforcement. Either wire it in
  (with its `perf_baseline.json`) or stop presenting it as part of the suite.
- **Third-party attribution is a release step, not an idea.** A release ships a
  third-party attribution list covering the dependency graph, sourced from a per-crate
  licence/FFI inventory, and a new dependency that introduces transitive unsafe is named
  there (the obligation `AGENTS.md` records). The `LICENSE` Notices clause
  (`LICENSE:71-79`) is the model: whoever receives a copy gets the terms and the
  `Required Notice:` lines. The per-crate inventory this used to be read from —
  `.planning/archive/DEPENDENCIES.md` — was deleted in the 2026-09-30 reset, so the list
  must be **regenerated from the dependency graph** (`cargo-about`/`cargo-deny` output)
  rather than copied from a file that no longer exists.
- **Done-condition.** A release dry-run produces the binary + `SHA256SUMS` + SBOM +
  third-party notices + a build attestation; the release notes document **how a user
  verifies that attestation**; new gates are visible in CI.

**Untrusted-input rigor.** `structure` executes agent-supplied Tree-sitter queries;
`query` executes agent SQL under a read-only connection, a subquery wrapper, and an
authorizer hook. These are the most interesting surfaces in the threat model and neither
has a fuzz target (there is no `fuzz/` crate). Mutation testing covers four files,
Linux-only.

- A `fuzz/` crate (cargo-fuzz) with three targets: Tree-sitter query strings fed to
  `structure`; SQL fed to `query`, including authorizer-bypass attempts; JSON-RPC frame
  parsing. Seed from `examples/queries/*` and `src/store/schema.sql`. Nightly/scheduled
  workflow, not per-PR.
- Adversarial authorizer tests as permanent unit tests (`ATTACH`, `PRAGMA`,
  `load_extension`, writes through CTEs/subqueries, comment/`;` splicing, multiple
  statements) asserting rejection, not just "no rows".
- Widen mutation testing or document why the narrower scope is intentional.
- A synthetic large-repo soak test that stays inside `max_sync_wall_clock` (nothing
  exercises a 10k-file import except the budget that would abort it).
- **Done-condition.** The fuzz targets build and run clean for a bounded time (scheduled,
  not per-PR); the adversarial authorizer tests are green and assert rejection rather than
  only "no rows"; the large-repo soak test completes inside `max_sync_wall_clock`.

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
- **End-to-end forced-skip test on the reconcile path is not achievable today** —
  `ReconcileOptions::for_sync` reads the process-wide `process_limits()` `OnceLock`, so a
  tiny `max_file_bytes` cannot be injected, and only the full-publish path applies
  `ProjectConfig` overrides. This is independent confirmation that **project config is
  ignored on the reconcile path** — a real asymmetry worth fixing (it is also why the
  skip-aware reconcile test has to sit at the `sweep_tree`/`apply_sweep_report` seam).
- **`extraction_completeness` is still read by no tool except `report`'s failure rows** —
  the completeness envelope across `report`/`leads`/`health` remains Phase 2.
