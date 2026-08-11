#!/usr/bin/env bash
# Performance regression gate. Runs the four criterion benches with a
# reduced sample size (fast enough for CI), parses criterion's median
# estimates, and fails when any bench regresses more than the threshold
# against the committed baseline (.planning/perf_baseline.json). The same
# policy as the coverage gate: the gated numbers are printed explicitly, so
# there is never a silent failure.
#
# The baseline is machine-class specific — re-record it deliberately when
# the hardware changes (see the "CI regression gate" section of
# .planning/PERFORMANCE.md):
#
#   bash tools/check_performance.sh --record
#
# Usage: bash tools/check_performance.sh [--record] [threshold-percent]
#   --record         regenerate .planning/perf_baseline.json from this run
#   threshold        regression threshold percent (default 20)
set -euo pipefail

record=0
[ "${1:-}" = "--record" ] && record=1 && shift
threshold="${1:-20}"

baseline_file=".planning/perf_baseline.json"
results_dir="target/criterion"

if [ "$record" -eq 0 ] && [ ! -f "$baseline_file" ]; then
    echo "FAIL: no baseline at $baseline_file; run 'bash tools/check_performance.sh --record' first" >&2
    exit 1
fi

# Fresh results only — stale estimates from earlier runs must not leak in.
rm -rf "$results_dir"

cargo bench --locked \
    --bench discovery --bench parsing --bench search --bench sync \
    -- --sample-size 10 --warm-up-time 1 --measurement-time 3

python3 - "$baseline_file" "$threshold" "$record" <<'PY'
import json
import os
import sys
from pathlib import Path

baseline_path, threshold_pct, record = sys.argv[1], float(sys.argv[2]), sys.argv[3] == "1"

# Load the existing baseline when present: comparison needs it, and
# --record preserves the recorded budgets from it.
old_benches = {}
if os.path.exists(baseline_path):
    old_benches = json.load(open(baseline_path)).get("benches", {})

def _fmt(ns):
    if ns >= 1e9:
        return "{:.2f} s".format(ns / 1e9)
    if ns >= 1e6:
        return "{:.2f} ms".format(ns / 1e6)
    if ns >= 1e3:
        return "{:.2f} us".format(ns / 1e3)
    return "{:.0f} ns".format(ns)

# Collect the median point estimate (nanoseconds) for every benchmark that
# just ran, keyed by criterion group/function.
new_benches = {}
for estimates in Path("target/criterion").rglob("new/estimates.json"):
    func_dir = estimates.parent.parent
    group_dir = func_dir.parent
    key = "{}/{}".format(group_dir.name, func_dir.name)
    data = json.load(open(estimates))
    new_benches[key] = data["median"]["point_estimate"]

if not new_benches:
    print("FAIL: no criterion estimates found under target/criterion", file=sys.stderr)
    sys.exit(1)

if record:
    benches = {}
    for key, median in sorted(new_benches.items()):
        budgets = {"budget_ns": old_benches.get(key, {}).get("budget_ns")}
        benches[key] = dict(median_ns=median, **budgets)
    out = {
        "machine": "current run (re-recorded {})".format(Path.cwd()),
        "threshold_percent": threshold_pct,
        "benches": benches,
    }
    with open(baseline_path, "w") as f:
        json.dump(out, f, indent=2)
        f.write("\n")
    print("recorded {} benchmarks to {}".format(len(benches), baseline_path))
    sys.exit(0)

failures = []
rows = []
warnings = []
for key in sorted(new_benches):
    if key not in old_benches:
        warnings.append("bench {} is not in the baseline -- run 'check_performance.sh --record' to add it".format(key))
for key in sorted(old_benches):
    if key not in new_benches:
        failures.append(
            "{key}: produced no measurements this run (did it skip?) -- the gate cannot check a bench that did not run".format(key=key)
        )
for key, new_median in sorted(new_benches.items()):
    old_entry = old_benches.get(key)
    if old_entry is None:
        rows.append((key, None, new_median, None, "new"))
        continue
    old_median = old_entry["median_ns"]
    ratio = new_median / old_median if old_median else float("inf")
    budget = old_entry.get("budget_ns")
    verdict = "ok"
    if ratio > 1 + threshold_pct / 100:
        verdict = "REGRESSED"
        failures.append(
            "{key}: {ratio:.1%} slower than baseline".format(key=key, ratio=ratio - 1)
        )
    if budget is not None and new_median > budget:
        verdict = "OVER BUDGET"
        failures.append(
            "{key}: {ms:.1f} ms exceeds the {budget_ms:.0f} ms budget".format(
                key=key, ms=new_median / 1e6, budget_ms=budget / 1e6
            )
        )
    rows.append((key, old_median, new_median, ratio, verdict))

print("perf regression check (threshold {:.0f}%, baseline: {})".format(threshold_pct, baseline_path))
print("{:<32} {:>12} {:>12} {:>7}  {}".format("bench", "baseline", "measured", "ratio", "verdict"))
for key, old_median, new_median, ratio, verdict in rows:
    if old_median is None:
        print("{:<32} {:>12} {:>12} {:>7}  {}".format(key, "-", _fmt(new_median), "-", verdict))
    else:
        print(
            "{:<32} {:>12} {:>12} {:>6.2f}x  {}".format(
                key, _fmt(old_median), _fmt(new_median), ratio, verdict
            )
        )

for warning in warnings:
    print("WARN: " + warning)
if failures:
    for failure in failures:
        print("FAIL: " + failure)
    sys.exit(1)
print("PASS: no benchmark regressed more than {:.0f}%".format(threshold_pct))
PY
