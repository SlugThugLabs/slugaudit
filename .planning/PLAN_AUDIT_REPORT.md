# SlugAudit Rust Implementation Plan Audit

Status: in progress — phase-by-phase review

## Audit scope

This audit evaluates whether the implementation plan, if followed literally,
would produce a product that could pass a senior engineering review. It does
not treat intended architecture as implemented behavior. It records confirmed
plan gaps, avoids inventing evidence, and adds corrections to the master plan
after the phase reviews are complete.

The audit is being performed one phase at a time. Individual notes are stored
under `.planning/plan-audit/`.

## Phase status

| Phase | Note | Status |
|---|---|---|
| 0 — Freeze boundary and baseline | `plan-audit/PHASE-00.md` | Audited — FAIL |
| 1 — Typed evidence contract | `plan-audit/PHASE-01.md` | Audited — FAIL |
| 2 — SQLite store | `plan-audit/PHASE-02.md` | Pending |
| 3 — Discovery and synchronization | `plan-audit/PHASE-03.md` | Audited — FAIL |
| 4 — Tree-sitter language pack | `plan-audit/PHASE-04.md` | Audited — FAIL |
| 5 — Evidence queries and search | `plan-audit/PHASE-05.md` | Audited — FAIL |
| 6 — Dependency relationships | `plan-audit/PHASE-06.md` | Audited — FAIL |
| 7 — MCP transport and tools | `plan-audit/PHASE-07.md` | Audited — FAIL |
| 8 — Findings and freshness | `plan-audit/PHASE-08.md` | Audited — FAIL |
| 9 — Performance and concurrency | `plan-audit/PHASE-09.md` | Audited — FAIL |
| 10 — Adversarial testing | `plan-audit/PHASE-10.md` | Audited — FAIL |
| 11 — Quality automation and CI | `plan-audit/PHASE-11.md` | Audited — FAIL |
| 12 — End-to-end acceptance | `plan-audit/PHASE-12.md` | Audited — FAIL |
| 13 — Documentation and handoff | `plan-audit/PHASE-13.md` | Audited — FAIL |

## Consolidated score

| Area | Score | Honest assessment |
|---|---:|---|
| Architecture | 5/10 | Good intended boundaries, but transaction, parser, revision, and MCP contracts are not sufficiently enforced. |
| Maintainability | 4/10 | The small-file rule is strong, but the plan leaves too many decisions implicit and delays onboarding artifacts. |
| Readability | 6/10 | Clear prose and sequencing; repeated “bounded/current/safe” terms lack measurable definitions. |
| Performance | 3/10 | Parallelism and benchmarks are planned, but no budgets, baselines, or cache/download policy are acceptance criteria. |
| Security | 5/10 | The threat boundary is reasonable, but cache integrity, path/snapshot races, resource limits, protocol framing, and dependency policy need concrete gates. |
| Testability | 4/10 | Many test ideas exist, but the plan lacks a failure matrix, golden fixture contract, fuzzing, and early harness enforcement. |
| Error checking | 3/10 | Error cases are listed inconsistently; typed error ownership and public mappings are not defined end-to-end. |
| Logging/tracing | 2/10 | Logging is deferred and lacks a field-level event contract, redaction rules, and coverage requirements. |

## Consolidated verdict

FAIL as written. Following the plan literally could produce a functioning
prototype, but it would not reliably produce a top-company senior-reviewable
service because key invariants remain optional or undefined. The plan becomes
potentially acceptable only after the correction block below is applied and
validated.

## Top ten plan smells

These are plan-level smells, not invented function findings. The Rust project
does not yet contain the functions being planned.

1. `IMPLEMENTATION_PLAN.md:Task 1.2` — one `ParserStatus` mixes resource and
   parse state; high.
2. `IMPLEMENTATION_PLAN.md:Task 2.1` — table list without relational
   constraints; critical.
3. `IMPLEMENTATION_PLAN.md:Task 2.2/2.3` — transaction ownership ambiguity;
   critical.
4. `IMPLEMENTATION_PLAN.md:Task 3.4` — atomic publish without a stable
   read-side revision handle; critical.
5. `IMPLEMENTATION_PLAN.md:Task 4.1` — 306-language count treated as a
   capability guarantee; high.
6. `IMPLEMENTATION_PLAN.md:Task 4.3` — raw AST fallback without storage and
   response budgets; high.
7. `IMPLEMENTATION_PLAN.md:Phase 5` — “bounded” search without FTS/scan,
   ordering, cancellation, or latency contract; high.
8. `IMPLEMENTATION_PLAN.md:Task 7.1` — MCP SDK and wire contract selected too
   late; high.
9. `IMPLEMENTATION_PLAN.md:Phase 9` — performance work without SLOs or
   reproducible baselines; high.
10. `IMPLEMENTATION_PLAN.md:Phase 11` — core quality enforcement deferred
    until late in the project; high.

There are no honest function-level cyclomatic rankings yet: the current Rust
production code contains only `src/main.rs::main`, with complexity 1, and it is
a placeholder. The plan must require complexity measurement after each phase.

## Testing, error, and observability synthesis

Realistic current coverage of implemented product logic: 0%. The plan itself
mentions many future tests, but Phase 0 can currently pass without a harness.
Critical missing cross-phase tests are:

- stable snapshot under file mutation during parsing and publication;
- parser cache/download/corruption/offline behavior;
- capability differences across the 306-language catalog;
- revision-bound reads;
- SQLite failure/migration/lock/disk-full behavior;
- FTS/search atomicity and cancellation;
- MCP schema/framing/cancellation/progress;
- stale finding lifecycle after source and contract changes;
- deterministic sequential versus parallel output;
- golden end-to-end fixture replay.

Realistic current error-checking coverage: 0%. Every public error needs a
typed owner, a redacted log event, a protocol/database behavior, and a test.

Realistic current logging/tracing coverage: 0%. Required event families are
startup, MCP request, root validation, sync/discovery, lock wait, parser
cache/load/process, normalization, store transaction, search, graph
resolution, finding transition, and shutdown. Logs must go to stderr and omit
source content, search patterns, secrets, and finding descriptions by default.

## User experience assessment

TUI rendering bugs and keybinding conflicts are not applicable because this is
an MCP stdio service, not a terminal UI. The plan must explicitly say so.

Applicable UX failures are currently underplanned:

- slow startup caused by parser discovery/download before the first request;
- no progress or status during a long first sync;
- no distinction between waiting, syncing, parser failure, and empty evidence;
- no cancellation when the AI stops waiting;
- protocol corruption if progress/logs reach stdout.

## “Assume production failure” prediction

The first likely failure is Phase 3/4 interaction: a large or mixed-language
repository triggers on-demand parser downloads while files are changing. The
plan, as written, can hash one state, parse another, publish partial/ambiguous
evidence, and still expose a revision without a final rehash and capability
record. The AI then receives evidence that looks current but is not a stable
snapshot.

The second likely failure is Phase 7: a long sync or parser download blocks the
stdio request loop, gives no progress/cancellation signal, or emits a log on
stdout and corrupts MCP framing.

## Senior-review verdict

Would the plan pass a top-company senior engineering review today? No.

Why: it is a strong outline, but not yet an executable production standard.
Too many critical words—“safe,” “bounded,” “current,” “where practical,”
“supported,” and “atomic”—lack exact invariants and failure tests. A senior
reviewer would reject it until the correction block is applied.

## Required correction block

Before implementation proceeds beyond the baseline, amend the master plan to:

1. Add Phase 0 executable MCP/baseline, test harness, README, CI, source-size,
   no-unsafe, dependency, logging, startup, and decision-log gates.
2. Split parser availability, parse outcome, completeness, and evidence origin.
3. Define typed source identity, span units, hashes, timestamps, IDs, ordering,
   limits, truncation, and missing-span provenance.
4. Define the complete SQLite schema, revision ownership, transaction owner,
   journal/lock policy, content retention, FTS, migrations, and failure model.
5. Add stable snapshot final rehash and read-side verified revision handles.
6. Add the language capability matrix, cache integrity/offline policy, ABI
   tests, alias normalization, parser reuse, and process-versus-raw-parser
   boundary from the upstream language-pack documentation.
7. Define search/graph budgets, ordering, cancellation, ambiguity, and
   provenance.
8. Move the MCP wire contract and SDK probe earlier; define schemas, errors,
   progress, framing, concurrency, and shutdown.
9. Define finding lifecycle, contract-version invalidation, and make sync unable
   to create findings.
10. Add performance SLOs, reproducible benchmarks, backpressure, and regression
    thresholds.
11. Add failure matrices, fuzz/property tests, resource-limit tests, and
    deterministic replay.
12. Define CI policies, coverage thresholds, dependency exceptions, lockfile,
    and toolchain enforcement from Phase 0.
13. Add golden fixture acceptance thresholds and explicit zero-skipped critical
    tests.
14. Make documentation incremental and add docs/schema/test drift checks.

## Consolidated correction log

The correction block has been applied to `IMPLEMENTATION_PLAN.md` after the
phase findings were recorded. The phase notes remain the evidence trail for
why each amendment exists.

## Final second-read disposition

The corrected plan was read again after the phase findings and correction block
were applied. One additional issue was found and corrected: the original
absolute physical-line rule could force artificial fragmentation and count
comments against the budget. It is now a hard target of fewer than 200 code
lines, excluding comments and blanks. Cohesive files from 200–300 code lines
may be approved with documented architectural reasoning; files over 300 block
implementation and require the user's review.

No other new contradiction was found in the second read. The plan now has the
necessary guardrails to prevent the known bad designs from being forced by the
specification. This is a plan-level PASS WITH CONDITIONS, not an implementation
pass: the conditions are the phase gates and validations in the amended master
plan.
