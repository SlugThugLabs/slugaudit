# Plan Audit — Phase 5

Status: audited — FAIL

## Verdict

The query surface is bounded, but the plan does not define whether search uses
SQLite FTS5 or scans source, how regex and literal results share ordering, or
how evidence queries prevent mixed revisions. Token efficiency is discussed as
a limit but not as a measurable response budget.

## Findings

1. Literal search has no case-folding/Unicode normalization policy.
2. Regex search lacks a time/memory budget and maximum file scan policy.
   Rust’s regex engine avoids catastrophic backtracking but does not make
   unlimited scanning acceptable.
3. Search indexing and content replacement are not part of the Phase 2 schema.
4. Result ordering is unspecified, so repeated AI calls can receive unstable
   evidence.
5. Read-file line bounds do not define CRLF handling, invalid UTF-8, or whether
   line numbering is source-byte or displayed-text based.
6. Evidence queries do not state which fields are indexed, which are bounded,
   or how partial results are disclosed.
7. Every query should bind a verified revision, but this is only a Phase 1
   correction and is not repeated as a hard query invariant here.
8. No cancellation behavior exists for a long search or retrieval.
9. There is no per-tool latency or response-byte target.
10. Search over source and search over structured evidence are not clearly
    distinguished, risking surprising result semantics.

## Required corrections

Choose FTS5 or document a bounded scan decision. Define Unicode, ordering,
limits, cancellation, revision binding, response byte budgets, and separate
source-content versus structured-evidence search semantics.

## Testing / logging

Add large-file, Unicode, invalid UTF-8, regex-limit, cancellation, stable
ordering, FTS atomic replacement, stale revision, and response-budget tests.
Log query kind, revision, result count, truncation, duration, and error class;
do not log patterns by default because patterns may contain secrets.
