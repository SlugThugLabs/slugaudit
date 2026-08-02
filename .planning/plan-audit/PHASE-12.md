# Plan Audit — Phase 12

Status: audited — FAIL

## Verdict

The end-to-end flow is the right shape but remains subjective. “Fixture
workflow succeeds” does not specify exact expected counts, response schemas,
latency, parser capability, failure behavior, or acceptance thresholds.

## Findings

1. Fixture language selection and expected evidence are not versioned.
2. No golden manifest/evidence expectations are required.
3. No exact MCP schema assertions are required for each tool.
4. No startup/first-sync/query performance threshold is enforced.
5. No test verifies an unsupported or partially supported grammar is honestly
   reported.
6. No test proves all non-binary files remain searchable.
7. No test proves no automated risk/finding output exists.
8. No test proves behavior after process restart and database reopen.
9. Release gate lists tools but not pass thresholds or artifact retention.
10. Skipped tests can still leave the overall acceptance ambiguous.

## Required corrections

Version a fixture contract with golden manifest, evidence counts/statuses,
schemas, latency budgets, restart behavior, partial-language expectations, and
explicit zero-skipped critical tests. Record all release artifacts.
