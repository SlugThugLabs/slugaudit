#!/usr/bin/env bash
# Regenerates every Phase 9 golden fixture from the current tools.
# Usage: tests/golden/phase9/generate.sh   (from anywhere)
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"
GOLDEN_PHASE9_RECORD=1 cargo test --lib golden_phase9 --all-features --locked -- --test-threads=1
