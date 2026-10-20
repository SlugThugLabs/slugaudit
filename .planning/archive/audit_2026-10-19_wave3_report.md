# Wave 3 — R4 production maintainability

Main before wave 3: `1b5eded`; wave 3 commits `0374d60`..`2dbd703`; module
map/mutation scope `d669848`. Date: 2026-10-19.

## Scope

The strict audit's 40 production R4 findings: 18 files over 200 code lines, 17
functions over 50 lines, and 5 functions over cyclomatic complexity 15. The
project's own source gate already passed; this wave applied the stricter
audit target with no source-limit exceptions.

## Result

- 7 disjoint refactor packages: sync publish/reconcile, sync core, watch/store,
  connect/project, tools query, tools surface, server/evidence/graph/model/util.
- 21 new internal modules under owned parents, all ≤200 code lines:
  `sync/hash_io.rs`, `sync/manager_health_support.rs`; five watch/store
  modules; three connect/project modules; five tools modules; six
  server/evidence/graph/util modules.
- Every audited production file is ≤200 code lines; every audited function is
  ≤50; every audited CC is ≤15. The old `slugaudit-line-exception` comments
  were removed from the refactored files (one stale annotation comment was
  removed in wave 4).
- Public paths were preserved with re-exports; error text, SQL, logging, and
  schema shapes were unchanged.
- `hash_file`/`stat_identity` moved to `sync/hash_io.rs`; the cargo-mutants
  scope was widened from four files to five in `.github/workflows/quality.yml`
  and AGENTS.md, because those functions are the correctness core.

## Validation

Full gate at `2dbd703` + `d669848`:

- fmt, clippy, deny, machete, source limits, duplicate checks, producer
  coverage, governance, version: pass.
- **1,176 tests passed, 0 failed, 1 ignored.**
- Coverage **93.27%** (11,661 / 12,503; gate 83%).
- Docs drift and MCP surface: pass (surface re-recorded later in wave 4 for the
  doc comments).
- Performance: pass, ratios 0.97x–1.06x against the 20% threshold.
- Production files over 200 code lines: **0**.
- clippy `too_many_lines` at 50: one remaining warning, a test function inside
  an inline test module in `tools/report_completeness.rs`; no production
  function warning.
- Agent cargo-crap reports: max CC ≤14 across the refactored packages.
