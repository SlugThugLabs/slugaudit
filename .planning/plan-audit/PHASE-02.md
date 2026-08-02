# Plan Audit — Phase 2

Status: audited

Scope: SQLite schema, transactions, migrations, and repositories in the plan.

## Verdict

FAIL as written. The plan chooses SQLite correctly for the current product
scope, but it does not specify the relational invariants needed to preserve
evidence integrity. A developer could implement materially different schemas,
transaction ownership, journal behavior, and deletion semantics while claiming
the phase passed.

## Phase score

| Area | Score | Reason |
|---|---:|---|
| Architecture | 5/10 | Store boundaries are named, but transaction ownership between services and repositories is undefined. |
| Maintainability | 5/10 | Repository separation is reasonable; schema and migration policy are too vague. |
| Readability | 6/10 | Tasks are easy to read but not precise enough to implement consistently. |
| Performance | 4/10 | Some indexes and busy timeout are named; FTS, WAL policy, growth, and batch behavior are absent. |
| Security | 5/10 | SQLite scope reduces exposure, but path permissions, cache/content limits, and migration safety are unspecified. |
| Testability | 5/10 | Rollback and concurrency are named but lack deterministic failure-injection methods. |
| Error checking | 5/10 | Constraint and rollback cases exist; corruption, disk-full, lock timeout, and migration failure do not. |
| Logging/tracing | 2/10 | Rollback is tested but no store event or redaction contract is defined. |

## Findings

### P2-01 — “Tables should cover” is not a schema contract

Location: Task 2.1.

The plan names entities but gives no columns, primary keys, foreign keys,
nullability, uniqueness, revision ownership, cascade behavior, or check
constraints. The store can therefore accept duplicate paths, orphan evidence,
cross-revision rows, or findings tied to nonexistent files.

Severity: critical.

Correction: add a schema contract table for every relation, including key,
foreign-key, uniqueness, deletion, and revision rules. Require the schema tests
to inspect those constraints, not just whether tables exist.

### P2-02 — Transaction ownership is ambiguous

Location: Task 2.2 and Task 2.3.

Repositories expose bulk replacement and transaction behavior, but the plan
does not say whether repositories start transactions or receive a transaction
owned by the sync service. If both layers begin transactions, nested SQLite
transactions will fail or require savepoints; if neither owns the transaction,
partial writes become possible.

Severity: critical.

Correction: make the sync/application layer the sole transaction owner for
multi-repository operations. Repositories must accept a transaction context and
never commit independently. Add a test proving one revision publish is one
atomic transaction.

### P2-03 — “Safe journal mode” is not a decision

Location: Task 2.2.

The plan does not choose WAL versus rollback journal, define busy timeout,
synchronous mode, checkpoint behavior, or behavior on filesystems where WAL is
unsafe or unsupported. “Safe” cannot be validated.

Severity: high.

Correction: specify SQLite pragmas, rationale, supported filesystem assumptions,
fallback behavior, and a test that reads while a writer is active. Record the
actual pragma values in startup diagnostics.

### P2-04 — File-content retention is unresolved

Location: Task 2.1: “file content or content references.”

The product requires cheap retrieval, but the plan leaves the central choice
open. Storing content in SQLite affects database size, locking, backup, and
query speed; storing references risks source disappearing or becoming stale.

Severity: critical.

Correction: choose one design for the first release. For SQLite-only local use,
store bounded source content in the database or explicitly define an immutable
content sidecar with hash verification. Add maximum file size and oversized-file
behavior.

### P2-05 — Revision ownership is not enforced at the row level

Location: tables list and Task 2.3.

The plan mentions revisions but does not say whether every derived row carries a
revision ID, whether the current revision is a project property, or how queries
prevent mixing rows from different revisions.

Severity: critical.

Correction: define revision scoping for every table and require repository
queries to take a verified revision. Add a foreign-key/unique strategy that
prevents mixed-revision evidence.

### P2-06 — Finding lifecycle is incomplete

Location: Task 2.3 validation 5.

The plan says a changed hash invalidates findings but does not define whether
they are deleted, marked stale, retained historically, or hidden from current
briefs. It also does not define duplicate finding identity or update behavior.

Severity: high.

Correction: define current versus historical findings, immutable source hash,
stable finding ID, duplicate policy, and exact query visibility rules.

### P2-07 — Migration policy is incomplete

Location: Task 2.1.

Idempotence is tested, but the plan does not define schema version storage,
forward-only migrations, interrupted migration recovery, unsupported future
versions, or whether downgrade is prohibited.

Severity: high.

Correction: require numbered forward migrations, a schema metadata row, an
exclusive migration lock/transaction, failure recovery, and explicit rejection
of newer schema versions.

### P2-08 — Database limits and disk failure behavior are absent

Location: Phase 2 as a whole.

An arbitrary repository can contain giant files, huge symbol counts, or many
parser nodes. The plan has no database-growth budget, disk-full behavior,
per-file write limit, or pruning policy for raw evidence.

Severity: high.

Correction: connect Phase 1 evidence budgets to store limits. Test disk-full or
write-failure simulation and prove the previous revision remains usable.

### P2-09 — SQLite locking and multi-process behavior are underspecified

Location: Task 2.2 validation 3; Phase 3 Task 3.4 validation 6.

“Concurrent reader behavior” and “two concurrent sync attempts” do not define
which process wins, how long a loser waits, whether duplicate parser work is
acceptable, or how a lock timeout is surfaced.

Severity: high.

Correction: define a per-project sync lock, lock acquisition timeout, winner/
loser behavior, stale lock recovery, and deterministic error metadata.

### P2-10 — Search storage is deferred without a schema decision

Location: Task 2.1 and Phase 5.

The product’s core value is fast searchable evidence, but the plan does not
decide whether SQLite FTS5 is used, how content is indexed, or how search rows
stay synchronized with file replacement.

Severity: high.

Correction: make FTS schema/update behavior part of Phase 2 or explicitly
justify a bounded scan. Test that content and FTS rows are replaced atomically.

## Code-smell audit

No Rust store functions exist, so function-level complexity cannot honestly be
ranked. The ten plan smells are:

1. Schema entity list without columns/constraints — critical.
2. Repository/transaction ownership ambiguity — critical.
3. “Safe journal mode” — undefined operational behavior; high.
4. “File content or content references” — unresolved primary design; critical.
5. Unenforced revision scoping — critical.
6. Hash-change finding invalidation without lifecycle — high.
7. Idempotent-only migration test — high.
8. No disk-full/database-growth policy — high.
9. Undefined concurrent sync winner/timeout — high.
10. Search index omitted from initial storage contract — high.

## Testing assessment

Realistic current coverage: 0% of store logic; no store code exists.

The named tests miss schema constraints, mixed-revision prevention, migration
interruption, newer-schema rejection, disk-full behavior, corrupted database,
lock timeout, FTS synchronization, content size limits, and finding history.

Correction: add schema introspection tests, failure injection, temporary
filesystem/database fixtures, concurrent-process tests, and FTS conformance
tests. Coverage must be measured, but invariants must also be asserted directly.

## Error-checking assessment

Realistic current coverage: 0%.

Required errors are missing for migration failure, unsupported schema version,
database corruption, lock timeout, busy timeout, disk full, oversized content,
constraint mapping, and failed rollback/checkpoint.

Correction: define typed store errors and map every one to a test and a public
operational response.

## Logging/tracing assessment

Realistic current coverage: 0%.

Missing store events include open/configuration, migration start/end, lock wait,
transaction begin/rollback/commit, row counts, database growth, and failures.
Logs must not include source content or finding descriptions by default.

## Security assessment

Concrete risks if the plan is followed literally:

- Unbounded content/evidence writes permit local disk exhaustion.
- Ambiguous content references could return stale source after a file changes.
- Missing row-level revision constraints can mix evidence from different syncs.
- Unspecified migration handling can leave a database partially upgraded.
- Undefined lock recovery can produce stale or concurrent revision publication.

## Required corrections to the master plan

Before Phase 3 begins, add:

1. Full schema table/column/key/constraint contract.
2. Sole transaction ownership in the sync/application layer.
3. Explicit SQLite pragma/journal/timeout policy.
4. A decision on content storage and oversized-file behavior.
5. Revision scoping and query invariants.
6. Finding current/history/duplicate lifecycle.
7. Numbered forward migrations and newer-version rejection.
8. Database growth, disk-full, corruption, and recovery behavior.
9. Per-project lock and concurrent-sync winner/timeout policy.
10. FTS5 or explicit bounded-search storage decision.
11. Store error matrix and structured store event tests.

## Phase 2 decision

FAIL. Do not begin Phase 3 until the master plan defines the store invariants
and transaction ownership precisely enough that two developers cannot produce
incompatible persistence behavior.
