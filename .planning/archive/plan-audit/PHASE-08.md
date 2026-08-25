# Plan Audit — Phase 8

Status: audited — FAIL

## Verdict

The AI-authored finding boundary is correct, but lifecycle and provenance are
not fully specified. A finding can become misleading if the source changes,
the evidence model changes, or the line range is no longer valid.

## Findings

1. Finding identity/deduplication is unspecified.
2. “Automatically purged” is not defined as delete, stale, or historical.
3. Parser/extraction contract changes can invalidate a finding even when source
   hash is unchanged; the plan only emphasizes file hash.
4. Line ranges need span semantics from Phase 1 and validation against current
   source.
5. Severity/category are AI inputs but have no length, normalization, or
   validation policy.
6. There is no edit/update/delete policy for AI findings.
7. No audit trail distinguishes AI-authored text from SlugAudit metadata.
8. The brief’s “evidence inventory” has no response budget or ordering rule.
9. The plan does not explicitly prohibit sync from creating findings in its
   schema/repository interfaces.
10. “Open finding” visibility across revisions is underspecified.

## Required corrections

Define immutable finding ID, source hash plus evidence-contract version,
current/stale/history states, bounded AI text, validation, provenance, and
brief ordering. Make sync interfaces incapable of inserting findings.

## Testing / logging

Test unchanged source/changed parser contract, changed line range, duplicate
submission, stale transition, historical retrieval, bounded text, and proof
that sync cannot write findings. Log finding ID and state transition, not full
descriptions.
