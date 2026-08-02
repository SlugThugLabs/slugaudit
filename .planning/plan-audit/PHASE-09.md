# Plan Audit — Phase 9

Status: audited — FAIL

## Verdict

The phase names concurrency and benchmarks but does not define success targets.
Parallelism can easily make parser cache access, SQLite writes, memory growth,
and deterministic ordering worse.

## Findings

1. No startup, unchanged-sync, first-sync, query-latency, or memory budgets.
2. No fixture size or machine baseline for comparable benchmarks.
3. Parser reuse and thread ownership are not specified.
4. One SQLite writer may become the bottleneck; no batching/queue policy exists.
5. Parallel failure aggregation and cancellation are unspecified.
6. Deterministic output is required but not defined for database insertion/order.
7. Parser downloads may serialize unexpectedly and block all workers.
8. No backpressure for huge repositories or raw evidence.
9. Benchmark results have no regression threshold.
10. Coverage/mutation tools can distort performance if run in the same gate.

## Required corrections

Define measurable budgets, fixture/machine protocol, parser-worker ownership,
bounded queues, cancellation, deterministic commit sorting, download warming,
and regression thresholds. Separate benchmark builds from correctness gates.

## Testing / logging

Compare sequential/parallel hashes and evidence, inject worker failure,
measure cold/warm cache, test queue pressure, and record parse/store/wait
durations. Fail only on documented statistically meaningful regressions.
