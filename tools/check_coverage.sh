#!/usr/bin/env bash
# Line-coverage gate. Reads the merged JSON line coverage — the same metric
# `cargo llvm-cov --fail-under-lines` compares (`totals.lines.percent`) —
# and fails the run if it is below the threshold. The measured number is
# printed so the gated value is never ambiguous: the `--summary-only` text
# table prints region-weighted columns that have been misread as line
# coverage before, and that printed table is not what the exit check
# compares against.
#
# Usage: bash tools/check_coverage.sh [threshold]
#   threshold: minimum line coverage percent (default 89).
set -euo pipefail

threshold="${1:-89}"

report="$(mktemp)"
err="$(mktemp)"
trap 'rm -f "$report" "$err"' EXIT

if ! cargo llvm-cov --all-targets --all-features --json >"$report" 2>"$err"; then
    echo "FAIL: cargo llvm-cov --json did not complete:" >&2
    cat "$err" >&2
    exit 1
fi

python3 - "$threshold" "$report" <<'PY'
import json
import sys

threshold = float(sys.argv[1])
cov = json.load(open(sys.argv[2]))

# Merge every data entry: newer cargo-llvm-cov versions can emit one entry
# per test binary, and the gate must measure the true merged coverage, not
# whichever binary happens to be first.
covered = 0
count = 0
for data in cov["data"]:
    lines = data["totals"]["lines"]
    covered += lines["covered"]
    count += lines["count"]
percent = (covered / count * 100) if count else 0.0

print(
    "line coverage: {:.2f}% ({} / {} lines, gate {}%)".format(
        percent, covered, count, threshold
    )
)
if percent < threshold:
    print("FAIL: line coverage {:.2f}% is below the {}% gate".format(percent, threshold))
    sys.exit(1)
PY
