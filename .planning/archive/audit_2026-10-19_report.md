# Rust Workspace Code-Quality Audit — SlugAudit

Repository: `/home/slugaudit`, `main` at `26ecdb4` (code tree at `f9544ca`; the working tree has
deleted repo docs, left untouched per instruction). Audit date: 2026-10-19.
**Files reviewed: 259 of 259 (50,325 lines).**

Method: every committed `.rs` file was read in full by one of 12 chunk auditors and checked
against R1–R12. Every cited violation was then re-opened by a separate verification pass.
The author pass produced 2,468 entries; 27 were dropped on re-check. Detector-derived R4 entries
(54 files, 45 functions, 10 complexity) are added. **2,550 confirmed violations**; 75 suspected
misses found during verification are recorded in Appendix C but not counted.

## 1. Inventory

### Crates / components

- `slugaudit-mcp-rust` v1.0.10 — root crate: library, `slugaudit` binary, nine development-only
  `check_*` gate binaries.
- `slugaudit-mcp-rust-fuzz` v0.0.0 — separate cargo workspace with three fuzz targets and support.
- `tests/` — five integration-test crates; `benches/` — seven criterion benches; `fuzz/` — four targets.

### Library modules (30, from `src/lib.rs`)

`cli`, `clock`, `connect`, `connect_agents`, `connect_cli`, `connect_exec`, `connect_guidance`,
`error_codes`, `evidence`, `graph`, `ignore_rules`, `install`, `install_place`, `menu`,
`menu_render`, `model`, `parse`, `probe`, `progress`, `project`, `prompt`, `resources`, `server`,
`server_runner`, `store`, `sync`, `tools`, `update`, `util`, `watch`.

### File and line totals

| Area | Files | Lines |
|---|---:|---:|
| `src/` core (non-`bin`) | 202 | 43,111 |
| `src/bin/` (dev gates) | 41 | 4,552 |
| `tests/` | 5 | 1,248 |
| `benches/` | 7 | 922 |
| `fuzz/` | 4 | 492 |
| **Total** | **259** | **50,325** |

Full per-file inventory: Appendix A.

## 2. Automatic detectors

| Detector / command | Exact result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | exit 0, 0 warnings |
| `cargo clippy ... -- -W clippy::redundant_clone -W clippy::implicit_clone -W clippy::clone_on_ref_ptr -W clippy::too_many_lines` (threshold 50 via `CLIPPY_CONF_DIR`) | 45 `too_many_lines` functions; **0** R9-lint warnings |
| `cargo test --lib --bins --tests --all-features --locked --no-fail-fast` | 1030 passed, 0 failed, 1 ignored |
| `cargo deny check` | advisories/bans/licenses/sources ok |
| `cargo machete` | 0 unused dependencies |
| `cargo run --quiet --bin check_coverage --locked` | line coverage **90.26%** (10,292 / 11,402), gate 83%, pass |
| `cargo crap --format json --lcov ... --min 0 --path src` | 947 functions; **10** with cyclomatic complexity >15; LCOV scope warning (207 analyzed, 161 in LCOV, 155 matched) |
| `check_source_limits` | PASS (project gate); **54** files >200 code lines under the audit’s stricter target |
| `check_no_duplicates` | PASS |
| `check_docs_drift` | **FAIL** — missing `.planning/PLAN.md`, `.planning/archive/DECISIONS.md`, `docs/CLIENTS.md`, `docs/TROUBLESHOOTING.md` |
| `check_producer_coverage` | PASS |
| `check_governance` | PASS |
| `check_mcp_surface` | **FAIL** — missing `.planning/archive/mcp_surface.json` |
| `check_version_consistency` | PASS |
| `RUSTFLAGS='-W missing_docs' cargo check --lib --bins` | 239 warnings / 212 unique sites |
| Project fast-check script | None exists; the seven gate binaries above were run instead |

The two failing gates are caused by the repo docs deleted in the working tree; they were not
restored, per instruction.

## 3. Rule table

Counts include tests for R1/R2 per this audit. R4/R9/R11 are detector-derived; the rest are
manual confirmed entries.

| Rule | Detector / command | Count | Production | Test/bench/fuzz | Dev-gate | Target | Pass/Fail |
|---|---|---:|---:|---:|---:|---:|---|
| R1 — no panics | manual + `rg` panic index | 1,921 | 3 | 1,918 | 0 | 0 | Fail |
| R2 — no silent failures | manual + `rg` discard index | 12 | 1 | 11 | 0 | 0 | Fail |
| R3 — no unsafe / waivers | `rg` `unsafe`/`allow`/`expect` | 17 | 8 | 0 | 9 | 0 | Fail |
| R4 — functions ≤50 / files ≤200 / CC ≤15 | clippy too_many_lines + check_source_limits + cargo-crap | 109 (45 fn + 54 file + 10 CC) | 40 | 60 | 9 | 0 | Fail |
| R5 — failure-path tests | manual per error/arm | 142 | 110 | 0 | 32 | 0 | Fail |
| R6 — truthful messages | manual | 27 | 22 | 1 | 4 | 0 | Fail |
| R7 — bounded inputs | manual | 15 | 5 | 0 | 10 | 0 | Fail |
| R8 — pub docs / external use | `rustc missing_docs` + manual grep | 258 | 258 | 0 | 0 | 0 | Fail |
| R9 — needless allocation | clippy redundant_clone/implicit_clone/clone_on_ref_ptr | 0 | 0 | 0 | 0 | 0 | **Pass** |
| R10 — duplicated logic >10 lines | manual | 26 | 3 | 23 | 0 | 0 | Fail |
| R11 — dependencies | cargo deny / machete | 0 | 0 | 0 | 0 | 0 | **Pass** |
| R12 — structured logs, no data | manual | 23 | 21 | 0 | 2 | 0 | Fail |

Dropped on re-check: 27 (R1 2 serde_json `Value` indexing; R2 2 Option/non-error; R7 2
already-bounded/not-reachable; R8 14 private-module or documented items; R10 1; R12 6 deliberate
one-shot or already-logged). The R8 count is higher than the 239-warning lint because it also
counts public items with no external reference; the lint is the authoritative missing-doc subset.

## 4. Violation ledger

The full verified ledger is Appendix B and `/tmp/audit2/ledger_verified_all.md`: **2,550 lines**,
one per confirmed violation, format `R# | path:line | scope | description`. No ellipses and no
“and others”. Appendix C holds 75 suspected misses found during verification; they are not counted.

## 5. Top 10 by risk

1. **R12 `src/server_runner.rs:191`** — a failing tool call logs `ErrorData.message`, which embeds
   file paths and, for a failed query, SQLite’s `near "<token>"` SQL fragment. Every failure lands
   in always-on stderr, so an agent’s mistyped SQL literal or private path is leaked.
2. **R7 `src/sync/manifest.rs:113`** — the whole upsert + deletion + skipped path set is serialized
   into one JSON SQL parameter with no count/size cap. A tree of millions of empty files is not
   bounded by `max_total_import_bytes` (discovery is time-bounded only) and can exhaust memory.
3. **R7 `src/tools/finding_read.rs:131`** — every finding in the session is returned in one
   response with no row/byte budget and no `next_offset`; descriptions up to 10,000 chars each can
   exceed MCP frame limits and lose the whole read.
4. **R7 `src/tools/structure.rs:217`** — `start_after_file` and `expected_revision` are unbounded
   MCP strings (the same `expected_revision` is capped at 256 bytes in `query`/`leads`); an
   oversized value is carried into SQL and comparisons.
5. **R7 `src/store/sql_functions.rs:53`** — `line_count`/`line_range` can scan a synthesized
   ~800 MB TEXT, and `line_range` copies the requested slice before the query response budget
   applies; only SQLite’s 1 GB value limit bounds it.
6. **R1 `src/sync/sample_batch.rs:266`** — `samples[idx]` uses a channel-supplied index with no
   in-function bounds guard; a sender/index-contract change panics the publish thread instead of
   failing closed.
7. **R6 `src/tools/report_completeness.rs:88` and `src/tools/report.rs:81`** — discovered files
   with non-UTF-8 names get no `files` row, so `index_partial` stays false and `file_count`
   understates discovery; an agent quoting the report concludes the index is complete when a file
   was dropped.
8. **R6 `src/tools/health.rs:373`** — pathless `health` derives the phase with no revision, so a
   synced project with a Healthy watcher is reported as `importing` (“the first sync has never
   succeeded”); the same path takes an arbitrary `HashMap` project (`:370`) despite the promise of
   the most-recently-touched one.
9. **R6 `src/connect.rs:81`** — `disconnect` prints “Done.” after `disconnect_agent` returns early
   when `config_home()` is `None` or for agents with no writable section/CLI route; the user is
   told the registration was removed while it remains.
10. **R5 `src/sync/manager_health.rs:299`** — the CAS baseline is read once before the barrier loop
    and reused; if iteration 1 publishes and a new event changes the same file, iteration 2 fails
    `RevisionError::StaleBaseline`, aborting the call and marking the watcher Desynced instead of
    reconciling in the same call. The test claiming to cover it cannot reach a publishing second
    iteration.

## 6. What could not be checked

- **Deleted repo docs.** `check_docs_drift` fails because `.planning/PLAN.md`,
  `.planning/archive/DECISIONS.md`, `docs/CLIENTS.md`, and `docs/TROUBLESHOOTING.md` are missing;
  `check_mcp_surface` fails because `.planning/archive/mcp_surface.json` is missing. They were not
  restored. The module-map/test-count checks and the MCP surface snapshot are therefore
  unverifiable in this tree.
- **R12 project policy.** The 2026-10-16 path-logging decision lives in the deleted DECISIONS file;
  verifiers used the decision’s known categories (from git history) and flagged only
  correlation-bearing logs, not the deliberate one-shot diagnostics.
- **Fuzz targets** were audited as source, not executed (nightly + sanitizer).
- **The 10k-file soak** was not run for this audit.
- **Coverage** is line coverage (90.26%), not branch coverage.
- **cargo-crap coverage scope**: 207 source files analyzed, 161 in LCOV, 155 matched; 52 files had
  no LCOV entry, so CRAP scores at those sites are pessimistic; `cyclomatic` values are
  independent of coverage.
- **R4 function length** is clippy’s line-span count with a 50-line threshold; file size is the
  project’s token-aware code-line counter; complexity is cargo-crap’s cyclomatic value.
- **R9** covers exactly the three named clippy lints; other needless allocations were not
  exhaustively reviewed.
- **R1 indexing** is a syntactic floor: library calls that can panic (`remove`, range slicing) were
  not exhaustively enumerated.
- **R2** distinguishes `Option` from `Result`; a discarded non-error `Option` is not counted.
- **R8** uses the rustc missing-docs lint for reachable items and grep for external references;
  14 author claims were dropped as private-module or already-documented.
- **Tests are in scope for R1/R2** per this audit; the project’s `cfg(test)` exemption explains why
  its own clippy/test gates pass while this count is large.

## Appendix A — full source-file inventory

All 259 committed `.rs` files, `wc -l` physical lines.

```
    313 benches/common/mod.rs
    157 benches/common/samples.rs
     61 benches/discovery.rs
    111 benches/memory.rs
     96 benches/parsing.rs
     96 benches/search.rs
     88 benches/startup.rs
     71 fuzz/fuzz_targets/query_sql.rs
     83 fuzz/fuzz_targets/request_arguments.rs
    114 fuzz/fuzz_targets/structure_query.rs
    224 fuzz/fuzz_targets/support.rs
    247 src/bin/check_coverage.rs
     49 src/bin/check_docs_drift/canonical_docs.rs
     72 src/bin/check_docs_drift/canonical_docs_tests.rs
    118 src/bin/check_docs_drift/main.rs
    102 src/bin/check_docs_drift/module_map.rs
     61 src/bin/check_docs_drift/module_map_tests.rs
     94 src/bin/check_docs_drift/test_file_count.rs
     52 src/bin/check_docs_drift/test_file_count_tests.rs
     29 src/bin/check_docs_drift/test_support.rs
     50 src/bin/check_governance/codeowners.rs
     64 src/bin/check_governance/main.rs
     34 src/bin/check_governance/required_files.rs
    108 src/bin/check_governance/tests.rs
    110 src/bin/check_mcp_surface/diff.rs
    141 src/bin/check_mcp_surface/main.rs
     38 src/bin/check_mcp_surface/surface.rs
    125 src/bin/check_mcp_surface/tests.rs
    149 src/bin/check_no_duplicates/commit_subjects.rs
     72 src/bin/check_no_duplicates/main.rs
    187 src/bin/check_no_duplicates/test_names.rs
     91 src/bin/check_no_duplicates/tests.rs
     48 src/bin/check_performance/args.rs
     48 src/bin/check_performance/bench.rs
    118 src/bin/check_performance/compare.rs
    133 src/bin/check_performance/estimates.rs
     61 src/bin/check_performance/format.rs
    127 src/bin/check_performance/main.rs
     43 src/bin/check_performance/record.rs
    279 src/bin/check_performance/tests.rs
    166 src/bin/check_producer_coverage/main.rs
    257 src/bin/check_producer_coverage/scan.rs
    123 src/bin/check_producer_coverage/tests.rs
     48 src/bin/check_source_limits/approval.rs
    112 src/bin/check_source_limits/counter.rs
    125 src/bin/check_source_limits/main.rs
    189 src/bin/check_source_limits/scanner.rs
    227 src/bin/check_source_limits/tests.rs
     68 src/bin/check_version_consistency/main.rs
    136 src/bin/check_version_consistency/manifests.rs
    140 src/bin/check_version_consistency/tests.rs
    111 src/bin/gate_common/args.rs
    181 src/cli.rs
    179 src/cli_tests.rs
    103 src/clock.rs
    425 src/connect_agents.rs
    183 src/connect_cli.rs
    304 src/connect_exec.rs
    178 src/connect_guidance.rs
    527 src/connect_registry_tests.rs
    168 src/connect.rs
    523 src/connect_tests.rs
    262 src/error_codes.rs
    246 src/evidence/calls/macro_calls.rs
    230 src/evidence/calls.rs
    128 src/evidence/calls_tests.rs
    195 src/evidence/generic_imports.rs
    209 src/evidence/generic_imports_tests.rs
    152 src/evidence/mod.rs
    129 src/evidence/normalize_builders.rs
    164 src/evidence/normalize_grammar_tests.rs
    366 src/evidence/normalize.rs
    471 src/evidence/normalize_tests.rs
    167 src/evidence/sql.rs
     80 src/graph/mod.rs
     71 src/graph/mod_tests.rs
     14 src/graph/reference.rs
    149 src/graph/reference_tests.rs
    157 src/graph/resolver/extract.rs
    115 src/graph/resolver/gate_tests.rs
    223 src/graph/resolver/generic.rs
     30 src/graph/resolver/js.rs
     48 src/graph/resolver/mod.rs
    122 src/graph/resolver/path_helpers.rs
    139 src/graph/resolver/proptest.rs
    129 src/graph/resolver/python.rs
     99 src/graph/resolver/registry.rs
     47 src/graph/resolve.rs
     80 src/graph/resolver/types.rs
    299 src/graph/resolve_rust.rs
    204 src/graph/resolve_rust_tests.rs
    238 src/graph/resolve_tests.rs
      9 src/graph/test_support.rs
    160 src/ignore_rules/build.rs
    222 src/ignore_rules.rs
    280 src/ignore_rules_tests.rs
     88 src/install_place.rs
    240 src/install.rs
    456 src/install_tests.rs
     49 src/lib.rs
    227 src/main.rs
    100 src/menu_render.rs
    193 src/menu.rs
     44 src/model/evidence.rs
    279 src/model/limits.rs
    189 src/model/limits_tests.rs
     20 src/model/mod.rs
    214 src/model/parser.rs
    172 src/model/profile.rs
    137 src/model/profile_tests.rs
     53 src/model/source.rs
    230 src/model/span.rs
    137 src/model/span_tests.rs
    177 src/model/system_ram.rs
     25 src/parse/language.rs
     19 src/parse/mod.rs
    163 src/probe.rs
     72 src/progress.rs
     99 src/project/activation_lock.rs
    354 src/project/activation.rs
    495 src/project/activation_tests.rs
    241 src/project/config.rs
     41 src/project/database_path.rs
     19 src/project/mod.rs
     89 src/project/root.rs
    161 src/prompt.rs
    112 src/resources.rs
    138 src/resources_tests.rs
    450 src/server.rs
    358 src/server_runner.rs
    373 src/server_runner_tests.rs
    353 src/store/connection.rs
    682 src/store/connection_tests.rs
     72 src/store/migrations.rs
    181 src/store/migrations_tests.rs
     37 src/store/mod.rs
    292 src/store/netfs.rs
    291 src/store/netfs_tests.rs
    149 src/store/sql_functions.rs
    137 src/store/sql_functions_tests.rs
    122 src/store/test_capture.rs
    202 src/sync/analyze.rs
     97 src/sync/analyze_tests.rs
    192 src/sync/classify.rs
    237 src/sync/discovery.rs
    238 src/sync/discovery_tests.rs
    306 src/sync/hash.rs
    344 src/sync/manager_health.rs
    290 src/sync/manager_meta.rs
    284 src/sync/manager_meta_tests.rs
    413 src/sync/manager_observability_tests.rs
    223 src/sync/manager_orchestration.rs
    219 src/sync/manager.rs
    536 src/sync/manager_tests.rs
    226 src/sync/manifest.rs
     49 src/sync/mod.rs
    242 src/sync/publish_acceptance_tests.rs
    338 src/sync/publish_attempt.rs
    114 src/sync/publish_cas.rs
    141 src/sync/publish_diff.rs
    318 src/sync/publish_edges_tests.rs
     18 src/sync/publish_log.rs
    408 src/sync/publish_mutation_tests.rs
    283 src/sync/publish_race_tests.rs
    127 src/sync/publish.rs
    277 src/sync/publish_tests.rs
     77 src/sync/race_hook.rs
    100 src/sync/reconcile/barrier.rs
     58 src/sync/reconcile/error.rs
     52 src/sync/reconcile/mod.rs
     40 src/sync/reconcile/options.rs
    322 src/sync/reconcile/pipeline.rs
    137 src/sync/reconcile/queries.rs
     81 src/sync/reconcile/reconcile_binary_tests.rs
    106 src/sync/reconcile/reconcile_ignore_tests.rs
    573 src/sync/reconcile/reconcile_tests.rs
     21 src/sync/reconcile/report.rs
     73 src/sync/revalidate.rs
    107 src/sync/revision_edges.rs
    120 src/sync/revision_files.rs
    286 src/sync/revision.rs
    131 src/sync/revision_skipped.rs
    136 src/sync/revision_tests.rs
    304 src/sync/sample_batch.rs
     35 src/sync/sample_batch_tests.rs
    321 src/sync/sample.rs
    510 src/sync/sample_tests.rs
    169 src/sync/sweep_reconcile.rs
    298 src/sync/sweep.rs
    497 src/sync/sweep_tests.rs
     73 src/sync/test_support.rs
    287 src/sync/timeout_tests.rs
    139 src/tools/context_race_tests.rs
    144 src/tools/context.rs
    257 src/tools/context_tests.rs
     36 src/tools/context_transactions.rs
     52 src/tools/context_transaction_tests.rs
    181 src/tools/finding_race_tests.rs
    140 src/tools/finding_read.rs
    167 src/tools/finding_read_tests.rs
    281 src/tools/finding.rs
    322 src/tools/finding_session_tests.rs
    437 src/tools/finding_tests.rs
     97 src/tools/health_database.rs
    432 src/tools/health.rs
    334 src/tools/health_state_tests.rs
    229 src/tools/health_tests.rs
    167 src/tools/leads_compact.rs
    226 src/tools/leads_mechanics.rs
    241 src/tools/leads_page.rs
    125 src/tools/leads_page_tests.rs
    243 src/tools/leads_query.rs
    386 src/tools/leads.rs
    605 src/tools/leads_tests.rs
    335 src/tools/mod.rs
     92 src/tools/page_size.rs
    240 src/tools/project_control.rs
    362 src/tools/project_control_tests.rs
    211 src/tools/query_clip.rs
    357 src/tools/query_clip_tests.rs
    293 src/tools/query_exec.rs
    328 src/tools/query_page_tests.rs
    236 src/tools/query.rs
    279 src/tools/query_safety_tests.rs
    589 src/tools/query_tests.rs
    149 src/tools/query_value.rs
    143 src/tools/redaction_tests.rs
    186 src/tools/report_completeness.rs
     63 src/tools/report_edges.rs
    341 src/tools/report.rs
    621 src/tools/report_tests.rs
    194 src/tools/structure_exec.rs
    349 src/tools/structure_limit_tests.rs
    100 src/tools/structure_page.rs
    259 src/tools/structure.rs
    197 src/tools/structure_source.rs
    311 src/tools/structure_tests.rs
     15 src/tools/test_support.rs
    177 src/update_fetch.rs
     90 src/update_fetch_tests.rs
    278 src/update.rs
    275 src/update_tests.rs
    393 src/util.rs
    321 src/util_tests.rs
     97 src/watch/event_filter.rs
    408 src/watch/manager_event_tests.rs
    487 src/watch/manager.rs
    105 src/watch/mod.rs
     68 src/watch/path.rs
    303 src/watch/scope.rs
    136 src/watch/scope_tests.rs
    242 src/watch/state.rs
    123 src/watch/tests.rs
    146 src/watch/types.rs
    286 tests/connect_tests.rs
    115 tests/gate_binaries.rs
    249 tests/large_repo_soak.rs
    155 tests/menu_tests.rs
    443 tests/stdio_protocol.rs
  50325 total
```


## Appendix B — verified violation ledger

Format: `R# | path:line | scope | description`.

## Chunk 1

R1 | src/cli_tests.rs:6 | test | `parse_args(std::iter::empty()).expect("valid arguments parse")` panics if the no-argument parse returns Err.
R1 | src/cli_tests.rs:34 | test | `parse_args(["serve"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:41 | test | `.unwrap_err()` on the parse of "foobar" panics if it unexpectedly parses.
R1 | src/cli_tests.rs:51 | test | `parse_args(["help"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:59 | test | `.expect_err(...)` on the "SErve" parse panics if it unexpectedly parses.
R1 | src/cli_tests.rs:67 | test | `parse_args(["connect"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:76 | test | `parse_args(["connect","claude"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:86 | test | `parse_args(["disconnect"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:95 | test | `parse_args(["disconnect","agy"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:106 | test | `parse_args(["remove","agy"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:116 | test | `parse_args(["install"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:125 | test | `parse_args(["install","--something"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:133 | test | `parse_args(["menu"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:142 | test | `parse_args(["menu","--wat"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:149 | test | `.unwrap_err()` on the parse of "bogus" panics if it unexpectedly parses.
R1 | src/cli_tests.rs:156 | test | `parse_args(["update"]).expect(...)` panics on an unexpected Err.
R1 | src/cli_tests.rs:163 | test | `.unwrap_err()` on the parse of "bogus" panics if it unexpectedly parses.
R1 | src/cli_tests.rs:174 | test | `parse_args([arg]).expect(...)` inside the version-spelling loop panics on an unexpected Err.
R1 | src/probe.rs:64 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/probe.rs:85 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/probe.rs:87 | test | `std::fs::write(&file, b"x").expect("write file")` panics if the fixture cannot be written.
R1 | src/probe.rs:106 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/probe.rs:120 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/probe.rs:122 | test | `usize::try_from(MAX_FOREIGN_TEXT_BYTES).expect("cap fits usize")` panics if the constant does not fit usize.
R1 | src/probe.rs:123 | test | `std::fs::write(&big, ...).expect("write oversized config")` panics if the fixture cannot be written.
R1 | src/probe.rs:136 | test | `TEST_ENV_LOCK.lock().expect("env lock")` panics if the process-wide test lock is poisoned.
R1 | src/probe.rs:137 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/probe.rs:153 | test | `TEST_ENV_LOCK.lock().expect("env lock")` panics if the process-wide test lock is poisoned.
R1 | src/error_codes.rs:196 | test | `std::fs::read_to_string(ARCHITECTURE.md).expect("read ARCHITECTURE.md")` panics if the document is missing or unreadable.
R1 | src/error_codes.rs:211 | test | `.find("<!-- error-codes:begin -->").expect(...)` panics if the begin marker is absent.
R1 | src/error_codes.rs:214 | test | `.find("<!-- error-codes:end -->").expect(...)` panics if the end marker is absent.
R1 | src/error_codes.rs:215 | test | `architecture[begin..end]` is data-dependent slicing with no guard that `begin <= end`; a document with the end marker before the begin marker panics.
R1 | src/error_codes.rs:227 | test | `budget_payload(Some(120_000)).expect(...)` panics if the budget payload is None.
R1 | src/error_codes.rs:229 | test | `payload["remedy"].as_str().expect(...)` panics if the remedy is not a string.
R1 | src/error_codes.rs:249 | test | `stale_page_payload().expect(...)` panics if the payload is None.
R1 | src/error_codes.rs:251 | test | `payload["remedy"].as_str().expect(...)` panics if the remedy is not a string.
R1 | src/util_tests.rs:25 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/util_tests.rs:27 | test | `std::fs::write(...).expect("write fixture")` panics if the fixture cannot be written.
R1 | src/util_tests.rs:29 | test | `read_file_capped_bytes(...).expect("at-cap file reads")` panics if the at-cap read fails.
R1 | src/util_tests.rs:36 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/util_tests.rs:38 | test | `std::fs::write(...).expect("write fixture")` panics if the fixture cannot be written.
R1 | src/util_tests.rs:40 | test | `.expect_err("over-cap file is refused")` panics if the over-cap read succeeds.
R1 | src/util_tests.rs:47 | test | `panic!("expected TooLarge, got {other:?}")` in the match arm panics on a wrong error variant.
R1 | src/util_tests.rs:53 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/util_tests.rs:56 | test | `.expect_err("missing file is an I/O error")` panics if the missing-file read succeeds.
R1 | src/util_tests.rs:62 | test | `panic!("expected Io, got {other:?}")` in the match arm panics on a wrong error variant.
R1 | src/util_tests.rs:71 | test | `mutex.lock().expect("unpoisoned at first")` panics if the mutex is already poisoned.
R1 | src/util_tests.rs:72 | test | `panic!("simulated panic inside the critical section")` is a deliberate fixture panic used to poison the mutex.
R1 | src/util_tests.rs:124 | test | `panic!("simulated panic {iteration}")` is a deliberate fixture panic used to poison the mutex repeatedly.
R1 | src/util_tests.rs:150 | test | `write!(out, "{byte:02x}").expect("write to String")` panics if the formatting write fails.
R1 | src/util_tests.rs:266 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/util_tests.rs:270 | test | `directory.path().canonicalize().expect("canonicalize tempdir")` panics if the fixture path cannot be canonicalized.
R1 | src/util_tests.rs:290 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/util_tests.rs:292 | test | `std::fs::write(...).expect("write oversized file")` panics if the fixture cannot be written.
R1 | src/util_tests.rs:295 | test | `.expect_err("an over-cap file must fail closed")` panics if the over-cap read succeeds.
R1 | src/util_tests.rs:306 | test | `tempfile::tempdir().expect("temp dir")` panics if the fixture directory cannot be created.
R1 | src/util_tests.rs:308 | test | `std::fs::write(...).expect("write at-cap file")` panics if the fixture cannot be written.
R1 | src/util_tests.rs:310 | test | `read_file_capped(...).expect("a file at the cap reads")` panics if the at-cap read fails.
R1 | src/util_tests.rs:318 | test | `serialized_len(&value).expect(...)` panics if serialization fails.
R1 | src/util_tests.rs:319 | test | `serde_json::to_string(&value).expect("to_string")` panics if serialization fails.
R1 | src/server_runner_tests.rs:47 | test | `result.expect("ok")` panics if `run_blocking` returns an error.
R1 | src/server_runner_tests.rs:60 | test | `.expect_err("must propagate the tool's error, not swallow it")` panics if the error is swallowed.
R1 | src/server_runner_tests.rs:68 | test | The closure body `panic!("boom")` is a deliberate fixture panic inside the blocking closure, used to test JoinError translation.
R1 | src/server_runner_tests.rs:110 | test | `.expect("task joins").expect("work succeeds")` chains two panicking expects on the join and work results.
R1 | src/server_runner_tests.rs:142 | test | `.expect("test runtime")` panics if the current-thread runtime cannot be built.
R1 | src/server_runner_tests.rs:153 | test | `.unwrap_or_else(...)` whose fallback is `panic!("field `{key}` missing from log line: {line}")`, panicking when the expected field is absent.
R1 | src/server_runner_tests.rs:228 | test | `.expect("call succeeds")` panics if the second `run_blocking` call fails.
R1 | src/server_runner_tests.rs:243 | test | `uuid::Uuid::parse_str(&session).expect(...)` panics if the logged session id is not a UUID.
R1 | src/server_runner_tests.rs:304 | test | `.expect_err("the tool's error must propagate")` panics if the error is swallowed.
R1 | src/server_runner_tests.rs:362 | test | `result.expect("query succeeds")` panics if the query call fails.
R1 | src/resources_tests.rs:30 | test | `.expect("the payload-key reference is served")` panics if the resource is not served.
R1 | src/resources_tests.rs:33 | test | `other => panic!("expected text contents, got {other:?}")` panics on non-text resource contents.
R1 | src/resources_tests.rs:57 | test | `rusqlite::Connection::open_in_memory().expect(...)` panics if the in-memory database cannot be opened.
R1 | src/resources_tests.rs:60 | test | `.expect("the canonical schema applies")` panics if the schema fails to apply.
R1 | src/resources_tests.rs:70 | test | `panic!("{uri} is not served as text")` panics when the recipe is not text contents.
R1 | src/resources_tests.rs:75 | test | `.unwrap_or_else(...)` with a `panic!` fallback, panicking when a recipe statement fails to compile.
R1 | src/resources_tests.rs:78 | test | `.unwrap_or_else(...)` with a `panic!` fallback, panicking when a recipe statement fails to run.
R1 | src/resources_tests.rs:79 | test | `rows.next().expect("stepping a recipe row")` panics if stepping the recipe result fails.
R1 | src/resources_tests.rs:108 | test | `read(resource.uri.as_str()).expect(...)` panics if a listed resource is unreadable.
R1 | src/resources_tests.rs:110 | test | `panic!("{} is not served as text", resource.uri)` panics when a listed resource is not text.
R1 | src/resources_tests.rs:123 | test | `read("slugaudit://schema.sql").expect("schema is served")` panics if the schema resource is absent.
R1 | src/resources_tests.rs:126 | test | `other => panic!("expected text contents, got {other:?}")` panics on non-text schema contents.
R3 | src/cli.rs:5 | prod | `#![allow(clippy::print_stdout)]` is a file-level lint waiver.
R3 | src/main.rs:2 | prod | `#![allow(clippy::print_stdout)]` is a file-level lint waiver.
R5 | src/cli.rs:140 | prod | Public error variant `ConnectError::EndOfInput` (built at `src/connect.rs:54`) has no test asserting that exact error; no test drives end-of-input.
R5 | src/cli.rs:142 | prod | Public error variant `ConnectError::BinaryPath` (built at `src/connect.rs:58`) has no test asserting that exact error.
R5 | src/cli.rs:144 | prod | Public error variant `ConnectError::HomeUnavailable` (built at `src/connect_exec.rs:180` and `:215`) has no test asserting that exact error.
R5 | src/cli.rs:174 | prod | Public `#[from]` variant `ConnectError::Json` (reachable via `?` at `src/connect_exec.rs:266` and `:301`) has no test asserting that exact error.
R5 | src/cli.rs:176 | prod | Public `#[from]` variant `ConnectError::Io` (built at `src/connect_exec.rs:225` and `:238`) has no test asserting that exact error.
R5 | src/server.rs:447 | prod | `read_resource`'s `resource_not_found` error for an unknown URI has no covering test; `resources_tests.rs` exercises `resources::read` returning None, never the handler's ErrorData and message.
R5 | src/server_runner.rs:136 | prod | The `acquire_owned` error arm (closed semaphore) of the tool-dispatch path is untested; no test closes the semaphore to exercise `internal_error(error.to_string())`.
R5 | src/server_runner.rs:152 | prod | The `JoinError` arm's covering test `server_runner_tests.rs:65` asserts only `result.is_err()`, never the specific "tool task failed: {error}" typed error the arm produces.
R6 | src/server.rs:397 | prod | The `health` tool description says a call without a path returns "the most-recently-touched project's watcher health"; `src/tools/health.rs:370` instead takes the first entry of `HashMap` iteration (`snapshot_all` at `src/watch/manager.rs:321-325`), so with two watched projects the caller can get an arbitrary one.
R6 | src/resources.rs:44 | prod | The `find-unhandled-panics` description claims to find unwrap/expect/panic "call sites", but the embedded recipe (`examples/queries/find_unhandled_panics.sql:10-14`) runs `payload LIKE '%expect%'` over the whole JSON, so a call made inside `fn handle_unexpected()` matches through the payload's `caller` key; this also contradicts `src/server.rs:94-95` ("always json_extract the payload, never LIKE on its text").
R6 | src/cli.rs:76 | prod | The help says `connect` will "Register this binary", but `connect_one` (`src/connect.rs:58-59`) first substitutes `~/.slugthug/slugaudit/slugaudit` via `prefer_slugthug_binary`, so an installed older binary is registered instead of the binary the user invoked; the same false claim is in the module docs at `src/cli.rs:2`.
R7 | src/server.rs:447 | prod | `ReadResourceRequestParams.uri` is unbounded and is formatted verbatim into the `resource_not_found` message; rmcp 3.5.1's `JsonRpcMessageCodec::new` sets `max_length: usize::MAX` (`transport/async_rw.rs:220-226`), so unlike tool inputs, which carry `MAX_PATH_BYTES` caps, a multi-megabyte URI is echoed back with no length check.
R8 | src/lib.rs:1:1 | prod | Crate-level documentation is missing (missing_docs gate).
R8 | src/main.rs:1:1 | prod | Crate-level documentation is missing (missing_docs gate).
R8 | src/cli.rs:16:15 | prod | Public field `Command::Connect::agent` has no doc comment.
R8 | src/cli.rs:18:18 | prod | Public field `Command::Disconnect::agent` has no doc comment.
R8 | src/cli.rs:138:5 | prod | Public variant `ConnectError::InvalidChoice` has no doc comment.
R8 | src/cli.rs:140:5 | prod | Public variant `ConnectError::EndOfInput` has no doc comment.
R8 | src/cli.rs:142:5 | prod | Public variant `ConnectError::BinaryPath` has no doc comment.
R8 | src/cli.rs:144:5 | prod | Public variant `ConnectError::HomeUnavailable` has no doc comment.
R8 | src/cli.rs:146:5 | prod | Public variant `ConnectError::UnknownAgent` has no doc comment.
R8 | src/cli.rs:148:5 | prod | Public variant `ConnectError::AgentCommand` has no doc comment.
R8 | src/cli.rs:148:20 | prod | Public field `ConnectError::AgentCommand::cli` has no doc comment.
R8 | src/cli.rs:148:33 | prod | Public field `ConnectError::AgentCommand::inner` has no doc comment.
R8 | src/cli.rs:150:5 | prod | Public variant `ConnectError::RemoveFailed` has no doc comment.
R8 | src/cli.rs:150:20 | prod | Public field `ConnectError::RemoveFailed::cli` has no doc comment.
R8 | src/cli.rs:150:33 | prod | Public field `ConnectError::RemoveFailed::status` has no doc comment.
R8 | src/cli.rs:152:5 | prod | Public variant `ConnectError::AddFailed` has no doc comment.
R8 | src/cli.rs:152:17 | prod | Public field `ConnectError::AddFailed::cli` has no doc comment.
R8 | src/cli.rs:152:30 | prod | Public field `ConnectError::AddFailed::status` has no doc comment.
R8 | src/cli.rs:154:5 | prod | Public variant `ConnectError::UnsupportedAgent` has no doc comment.
R8 | src/cli.rs:154:24 | prod | Public field `ConnectError::UnsupportedAgent::agent` has no doc comment.
R8 | src/cli.rs:154:39 | prod | Public field `ConnectError::UnsupportedAgent::reason` has no doc comment.
R8 | src/cli.rs:159:5 | prod | Public variant `ConnectError::SectionNotAnObject` has no doc comment.
R8 | src/cli.rs:159:26 | prod | Public field `ConnectError::SectionNotAnObject::section` has no doc comment.
R8 | src/cli.rs:164:5 | prod | Public variant `ConnectError::RootNotAnObject` has no doc comment.
R8 | src/cli.rs:164:23 | prod | Public field `ConnectError::RootNotAnObject::path` has no doc comment.
R8 | src/cli.rs:168:5 | prod | Public variant `ConnectError::InvalidConfig` has no doc comment.
R8 | src/cli.rs:169:9 | prod | Public field `ConnectError::InvalidConfig::path` has no doc comment.
R8 | src/cli.rs:171:9 | prod | Public field `ConnectError::InvalidConfig::source` has no doc comment.
R8 | src/cli.rs:174:5 | prod | Public variant `ConnectError::Json` has no doc comment.
R8 | src/cli.rs:176:5 | prod | Public variant `ConnectError::Io` has no doc comment.
R8 | src/cli.rs:56 | prod | Public const `USAGE` is documented but referenced only inside `cli.rs` and its own test module; no external reference.
R8 | src/error_codes.rs:64 | prod | Public const `ErrorCode::ALL` is documented but referenced only inside `error_codes.rs` and its own test module; no external reference.
R8 | src/error_codes.rs:75 | prod | Public method `ErrorCode::as_str` is documented but called only from `error_codes.rs` itself; no external reference.
R8 | src/server.rs:160 | prod | Public `SlugAuditServer::with_manager` is documented but called only inside `server.rs`; no external reference in `src/`, `tests/`, or `benches/`.
R8 | src/progress.rs:35:15 | prod | Public field `ProgressEvent::Started::phase` has no doc comment.
R8 | src/progress.rs:42:9 | prod | Public field `ProgressEvent::Sampling::phase` has no doc comment.
R8 | src/progress.rs:43:9 | prod | Public field `ProgressEvent::Sampling::current` has no doc comment.
R8 | src/progress.rs:44:9 | prod | Public field `ProgressEvent::Sampling::total` has no doc comment.
R8 | src/progress.rs:49:17 | prod | Public field `ProgressEvent::Completed::phase` has no doc comment.
R8 | src/progress.rs:59:5 | prod | Public trait method `ProgressSink::emit` has no doc comment.
R12 | src/main.rs:43 | prod | `init_logging` runs only after the command dispatch, so tracing warnings raised by CLI commands reach no subscriber and are dropped; e.g. `probe.rs:13`, `:30`, `:48` (documented as "logged with the source error"), `install.rs:166`, and `update.rs:213` lose their cause on the connect/install/update/menu paths.
R12 | src/main.rs:129 | prod | The `RUST_LOG` NotUnicode rejection interpolates `std::env::VarError`'s Display, which includes the raw environment value (verified: `environment variable was not valid unicode: "..."`), and the string is logged at `src/main.rs:174`; this puts an env value in the log.
R12 | src/main.rs:139 | prod | The `SLUGAUDIT_LOG_FORMAT` NotUnicode rejection interpolates the same Display, logging the raw environment value at `src/main.rs:174`.
R12 | src/server_runner.rs:191 | prod | The completion log emits `error.message`, which routinely carries user input: `src/tools/structure_source.rs:24` builds `"{file} has no indexed source content"`, `src/tools/finding.rs:237` the same, and `src/tools/query_exec.rs:290` passes rusqlite text such as `no such column: <agent identifier>`; this contradicts the file's own "SQL text ... never reach the log" contract at lines 62-64 and leaks paths/SQL fragments.

## Chunk 2

R1 | src/connect_registry_tests.rs:50 | test | `panic!` is an unconditional panic path: `Strategy::Unknown => panic!(`
R1 | src/connect_registry_tests.rs:90 | test | `expect` is an unconditional panic path: `find_agent(id).expect("agent").evidence,`
R1 | src/connect_registry_tests.rs:99 | test | `expect` is an unconditional panic path: `find_agent(id).expect("agent").evidence,`
R1 | src/connect_registry_tests.rs:107 | test | `expect` is an unconditional panic path: `find_agent(id).expect("agent").evidence,`
R1 | src/connect_registry_tests.rs:200 | test | `expect` is an unconditional panic path: `find_agent("claude").expect("claude").dialect,`
R1 | src/connect_registry_tests.rs:204 | test | `expect` is an unconditional panic path: `find_agent("gemini").expect("gemini").dialect,`
R1 | src/connect_registry_tests.rs:209 | test | `expect` is an unconditional panic path: `find_agent("hermes").expect("hermes").dialect,`
R1 | src/connect_registry_tests.rs:212 | test | `expect` is an unconditional panic path: `assert_eq!(find_agent("crush").expect("crush").dialect, Dialect::Crush);`
R1 | src/connect_registry_tests.rs:213 | test | `expect` is an unconditional panic path: `assert_eq!(find_agent("q").expect("q").dialect, Dialect::Q);`
R1 | src/connect_registry_tests.rs:215 | test | `expect` is an unconditional panic path: `find_agent("pi").expect("pi").dialect,`
R1 | src/connect_registry_tests.rs:219 | test | `expect` is an unconditional panic path: `find_agent("opencode").expect("opencode").dialect,`
R1 | src/connect_registry_tests.rs:224 | test | `expect` is an unconditional panic path: `find_agent("copilot").expect("copilot").dialect,`
R1 | src/connect_registry_tests.rs:229 | test | `expect` is an unconditional panic path: `find_agent("amp").expect("amp").dialect,`
R1 | src/connect_registry_tests.rs:233 | test | `expect` is an unconditional panic path: `find_agent("openhands").expect("openhands").dialect,`
R1 | src/connect_registry_tests.rs:238 | test | `expect` is an unconditional panic path: `find_agent("bob").expect("bob").dialect,`
R1 | src/connect_registry_tests.rs:243 | test | `expect` is an unconditional panic path: `find_agent("droid").expect("droid").dialect,`
R1 | src/connect_registry_tests.rs:251 | test | `expect` is an unconditional panic path: `let zed = find_agent("zed").expect("zed");`
R1 | src/connect_registry_tests.rs:265 | test | `expect` is an unconditional panic path: `find_agent("opencode").expect("opencode").shape,`
R1 | src/connect_registry_tests.rs:269 | test | `expect` is an unconditional panic path: `find_agent("opencode").expect("opencode").config,`
R1 | src/connect_registry_tests.rs:279 | test | `expect` is an unconditional panic path: `find_agent("goose").expect("goose").shape,`
R1 | src/connect_registry_tests.rs:283 | test | `expect` is an unconditional panic path: `let desktop = find_agent("claude-desktop").expect("claude-desktop");`
R1 | src/connect_registry_tests.rs:285 | test | `expect` is an unconditional panic path: `let resolved = desktop.config.resolve(home).expect("per-os config path");`
R1 | src/connect_registry_tests.rs:395 | test | `expect` is an unconditional panic path: `let remedy = a.guidance(Path::new("/bin/slugaudit")).expect("remedy");`
R1 | src/connect_registry_tests.rs:452 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/connect_registry_tests.rs:454 | test | `expect` is an unconditional panic path: `std::fs::write(&config, "{ not json").expect("write config");`
R1 | src/connect_registry_tests.rs:466 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/connect_registry_tests.rs:468 | test | `expect` is an unconditional panic path: `std::fs::write(&config, r#"{"mcpServers":{"slugaudit":{}}}"#).expect("write config");`
R1 | src/connect_registry_tests.rs:482 | test | `expect` is an unconditional panic path: `let home = tempfile::tempdir().expect("temp dir");`
R1 | src/connect_registry_tests.rs:483 | test | `expect` is an unconditional panic path: `let goose = find_agent("goose").expect("goose is registered");`
R1 | src/connect_registry_tests.rs:487 | test | `expect` is an unconditional panic path: `.expect("goose config path");`
R1 | src/connect_registry_tests.rs:488 | test | `expect` is an unconditional panic path: `std::fs::create_dir_all(config.parent().expect("config parent")).expect("mkdir");`
R1 | src/connect_registry_tests.rs:489 | test | `expect` is an unconditional panic path: `let over_cap = usize::try_from(crate::util::MAX_FOREIGN_TEXT_BYTES).expect("cap fits") + 1;`
R1 | src/connect_registry_tests.rs:490 | test | `expect` is an unconditional panic path: `std::fs::write(&config, vec![b' '; over_cap]).expect("write oversized config");`
R1 | src/connect_registry_tests.rs:501 | test | `expect` is an unconditional panic path: `let home = tempfile::tempdir().expect("temp dir");`
R1 | src/connect_registry_tests.rs:504 | test | `expect` is an unconditional panic path: `std::fs::write(home.path().join(".cursor"), b"x").expect("write file");`
R1 | src/connect_registry_tests.rs:505 | test | `expect` is an unconditional panic path: `let cursor = find_agent("cursor").expect("cursor is registered");`
R1 | src/connect_registry_tests.rs:517 | test | `expect` is an unconditional panic path: `let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/connect_registry_tests.rs:518 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/connect_registry_tests.rs:520 | test | `expect` is an unconditional panic path: `std::fs::write(&file, b"x").expect("write file");`
R1 | src/connect_tests.rs:22 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/connect_tests.rs:23 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/connect_tests.rs:25 | test | `expect` is an unconditional panic path: `fs::create_dir_all(&bin_dir).expect("bin dir");`
R1 | src/connect_tests.rs:26 | test | `expect` is an unconditional panic path: `fs::write(bin_dir.join("slugaudit"), b"#!fake").expect("fake binary");`
R1 | src/connect_tests.rs:31 | test | `expect` is an unconditional panic path: `prefer_slugthug_binary(&current).expect("stat succeeds"),`
R1 | src/connect_tests.rs:40 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/connect_tests.rs:49 | test | `expect` is an unconditional panic path: `prefer_slugthug_binary(&current).expect("stat succeeds"),`
R1 | src/connect_tests.rs:59 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:61 | test | `expect` is an unconditional panic path: `let agent = find_agent("cursor").expect("cursor agent");`
R1 | src/connect_tests.rs:66 | test | `expect` is an unconditional panic path: `connect_agent(&agent, binary, Some(home)).expect("connect cursor");`
R1 | src/connect_tests.rs:70 | test | `expect` is an unconditional panic path: `let content = fs::read_to_string(&cfg_path).expect("read config");`
R1 | src/connect_tests.rs:74 | test | `expect` is an unconditional panic path: `disconnect_agent(&agent, Some(home)).expect("disconnect cursor");`
R1 | src/connect_tests.rs:76 | test | `expect` is an unconditional panic path: `let after = fs::read_to_string(&cfg_path).expect("read config after");`
R1 | src/connect_tests.rs:84 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:86 | test | `expect` is an unconditional panic path: `let agent = find_agent("zed").expect("zed agent");`
R1 | src/connect_tests.rs:91 | test | `expect` is an unconditional panic path: `connect_agent(&agent, binary, Some(home)).expect("connect zed");`
R1 | src/connect_tests.rs:95 | test | `expect` is an unconditional panic path: `let content = fs::read_to_string(&cfg_path).expect("read zed settings");`
R1 | src/connect_tests.rs:99 | test | `expect` is an unconditional panic path: `disconnect_agent(&agent, Some(home)).expect("disconnect zed");`
R1 | src/connect_tests.rs:107 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:109 | test | `expect` is an unconditional panic path: `fs::create_dir_all(home.join(".cursor")).expect("mkdir .cursor");`
R1 | src/connect_tests.rs:116 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("bin dir");`
R1 | src/connect_tests.rs:118 | test | `expect` is an unconditional panic path: `fs::write(&script, format!("#!/bin/sh\nexit {exit_code}\n")).expect("write fake cli");`
R1 | src/connect_tests.rs:122 | test | `expect` is an unconditional panic path: `let mut perms = fs::metadata(&script).expect("metadata").permissions();`
R1 | src/connect_tests.rs:124 | test | `expect` is an unconditional panic path: `fs::set_permissions(&script, perms).expect("chmod fake cli");`
R1 | src/connect_tests.rs:133 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/connect_tests.rs:140 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/connect_tests.rs:141 | test | `expect` is an unconditional panic path: `let empty = tempfile::tempdir().expect("empty bin dir");`
R1 | src/connect_tests.rs:160 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:175 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:184 | test | `panic!` is an unconditional panic path: `other => panic!("expected AddFailed, got {other:?}"),`
R1 | src/connect_tests.rs:192 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:194 | test | `expect` is an unconditional panic path: `let agent = find_agent("cursor").expect("cursor agent");`
R1 | src/connect_tests.rs:198 | test | `expect` is an unconditional panic path: `fs::create_dir_all(cfg_path.parent().expect("parent")).expect("mkdir");`
R1 | src/connect_tests.rs:200 | test | `expect` is an unconditional panic path: `fs::write(&cfg_path, original).expect("write original");`
R1 | src/connect_tests.rs:207 | test | `expect` is an unconditional panic path: `let after = fs::read_to_string(&cfg_path).expect("read after");`
R1 | src/connect_tests.rs:220 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:222 | test | `expect` is an unconditional panic path: `let agent = find_agent(id).expect("agent");`
R1 | src/connect_tests.rs:224 | test | `expect_err` is an unconditional panic path: `.expect_err("connect must refuse");`
R1 | src/connect_tests.rs:240 | test | `panic!` is an unconditional panic path: `other => panic!("{id} must refuse automated registration, got {other:?}"),`
R1 | src/connect_tests.rs:250 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:252 | test | `expect` is an unconditional panic path: `let goose = find_agent("goose").expect("goose");`
R1 | src/connect_tests.rs:267 | test | `expect_err` is an unconditional panic path: `.expect_err("connect must refuse");`
R1 | src/connect_tests.rs:269 | test | `panic!` is an unconditional panic path: `panic!("expected an UnsupportedAgent refusal");`
R1 | src/connect_tests.rs:290 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:292 | test | `expect` is an unconditional panic path: `let agent = find_agent("omp").expect("omp");`
R1 | src/connect_tests.rs:295 | test | `expect` is an unconditional panic path: `connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).expect("connect omp");`
R1 | src/connect_tests.rs:296 | test | `expect` is an unconditional panic path: `let content = fs::read_to_string(home.join(".omp/agent/mcp.json")).expect("omp config");`
R1 | src/connect_tests.rs:301 | test | `expect` is an unconditional panic path: `disconnect_agent(&agent, Some(home)).expect("disconnect omp");`
R1 | src/connect_tests.rs:308 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:310 | test | `expect` is an unconditional panic path: `let warp = find_agent("warp").expect("warp");`
R1 | src/connect_tests.rs:313 | test | `expect` is an unconditional panic path: `connect_agent(&warp, Path::new("/bin/slugaudit"), Some(home)).expect("connect warp");`
R1 | src/connect_tests.rs:314 | test | `expect` is an unconditional panic path: `let content = fs::read_to_string(home.join(".warp/.mcp.json")).expect("warp config");`
R1 | src/connect_tests.rs:321 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:323 | test | `expect` is an unconditional panic path: `let agent = find_agent("pi").expect("pi");`
R1 | src/connect_tests.rs:326 | test | `expect` is an unconditional panic path: `connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home)).expect("connect pi via cli");`
R1 | src/connect_tests.rs:342 | test | `expect` is an unconditional panic path: `.expect("write recording cli");`
R1 | src/connect_tests.rs:346 | test | `expect` is an unconditional panic path: `let mut perms = fs::metadata(script).expect("metadata").permissions();`
R1 | src/connect_tests.rs:348 | test | `expect` is an unconditional panic path: `fs::set_permissions(script, perms).expect("chmod recording cli");`
R1 | src/connect_tests.rs:374 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:376 | test | `expect` is an unconditional panic path: `fs::create_dir_all(&home).expect("home");`
R1 | src/connect_tests.rs:378 | test | `expect` is an unconditional panic path: `fs::create_dir_all(&bin_dir).expect("bin dir");`
R1 | src/connect_tests.rs:382 | test | `expect` is an unconditional panic path: `let agent = find_agent(id).expect("agent");`
R1 | src/connect_tests.rs:384 | test | `expect` is an unconditional panic path: `connect_agent(&agent, Path::new("/bin/slugaudit"), Some(&home)).expect("connect");`
R1 | src/connect_tests.rs:386 | test | `expect` is an unconditional panic path: `let recorded = fs::read_to_string(&log).expect("args log");`
R1 | src/connect_tests.rs:393 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:395 | test | `expect` is an unconditional panic path: `let agent = find_agent("pi").expect("pi");`
R1 | src/connect_tests.rs:400 | test | `expect` is an unconditional panic path: `.expect("connect pi without cli");`
R1 | src/connect_tests.rs:405 | test | `expect` is an unconditional panic path: `disconnect_agent(&agent, Some(home)).expect("disconnect pi");`
R1 | src/connect_tests.rs:411 | test | `expect` is an unconditional panic path: `fs::remove_file(&config).expect("remove config before the fallback");`
R1 | src/connect_tests.rs:416 | test | `expect` is an unconditional panic path: `.expect("fall back after a rejected add");`
R1 | src/connect_tests.rs:428 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:429 | test | `expect` is an unconditional panic path: `let agent = find_agent("cursor").expect("cursor agent");`
R1 | src/connect_tests.rs:431 | test | `expect` is an unconditional panic path: `fs::create_dir_all(cfg_path.parent().expect("parent")).expect("mkdir");`
R1 | src/connect_tests.rs:432 | test | `expect` is an unconditional panic path: `fs::write(&cfg_path, "[]").expect("write array config");`
R1 | src/connect_tests.rs:437 | test | `panic!` is an unconditional panic path: `panic!("expected RootNotAnObject, got {result:?}");`
R1 | src/connect_tests.rs:441 | test | `expect` is an unconditional panic path: `fs::read_to_string(&cfg_path).expect("read"),`
R1 | src/connect_tests.rs:451 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("tempdir");`
R1 | src/connect_tests.rs:452 | test | `expect` is an unconditional panic path: `let agent = find_agent("cursor").expect("cursor agent");`
R1 | src/connect_tests.rs:454 | test | `expect` is an unconditional panic path: `fs::create_dir_all(cfg_path.parent().expect("parent")).expect("mkdir");`
R1 | src/connect_tests.rs:455 | test | `expect` is an unconditional panic path: `fs::write(&cfg_path, r#"{"mcpServers": []}"#).expect("write config");`
R1 | src/connect_tests.rs:460 | test | `panic!` is an unconditional panic path: `panic!("expected SectionNotAnObject, got {result:?}");`
R1 | src/connect_tests.rs:485 | test | `expect` is an unconditional panic path: `parse_choice(answer, 3).expect(answer),`
R1 | src/connect_tests.rs:501 | test | `expect` is an unconditional panic path: `let agent = find_agent("claude").expect("claude agent");`
R1 | src/connect_tests.rs:508 | test | `panic!` is an unconditional panic path: `panic!("expected RemoveFailed, got {result:?}");`
R1 | src/connect_tests.rs:515 | test | `expect` is an unconditional panic path: `let agent = find_agent("claude").expect("claude agent");`
R1 | src/connect_tests.rs:520 | test | `panic!` is an unconditional panic path: `panic!("expected AgentCommand, got {result:?}");`
R1 | src/install_tests.rs:12 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:14 | test | `expect` is an unconditional panic path: `run_install().expect("install succeeds");`
R1 | src/install_tests.rs:23 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:25 | test | `expect` is an unconditional panic path: `std::fs::write(&installed, b"the previous build").expect("plant a previous binary");`
R1 | src/install_tests.rs:31 | test | `expect` is an unconditional panic path: `std::fs::read(&previous).expect("the previous binary is retained"),`
R1 | src/install_tests.rs:44 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:46 | test | `expect` is an unconditional panic path: `std::fs::write(&target, b"the previous build").expect("write the binary to replace");`
R1 | src/install_tests.rs:47 | test | `expect` is an unconditional panic path: `std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");`
R1 | src/install_tests.rs:50 | test | `expect` is an unconditional panic path: `.expect("retaining succeeds")`
R1 | src/install_tests.rs:51 | test | `expect` is an unconditional panic path: `.expect("there was a binary to retain");`
R1 | src/install_tests.rs:59 | test | `expect` is an unconditional panic path: `.expect("stat the retained copy")`
R1 | src/install_tests.rs:70 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:75 | test | `expect` is an unconditional panic path: `.expect("retaining succeeds")`
R1 | src/install_tests.rs:84 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:85 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:91 | test | `expect` is an unconditional panic path: `\|\| assert_eq!(slugthug_home().expect("home resolves"), temp.path()),`
R1 | src/install_tests.rs:97 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:98 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:101 | test | `expect` is an unconditional panic path: `slugaudit_dir().expect("slugaudit dir resolves"),`
R1 | src/install_tests.rs:109 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:118 | test | `expect` is an unconditional panic path: `config_home().expect("config home resolves"),`
R1 | src/install_tests.rs:128 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:139 | test | `expect` is an unconditional panic path: `config_home().expect("config home resolves"),`
R1 | src/install_tests.rs:148 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:158 | test | `expect` is an unconditional panic path: `.expect("config home resolves")`
R1 | src/install_tests.rs:169 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:177 | test | `expect` is an unconditional panic path: `slugthug_home().expect("home resolves"),`
R1 | src/install_tests.rs:186 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:198 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:205 | test | `expect` is an unconditional panic path: `let source = running_binary().expect("running binary");`
R1 | src/install_tests.rs:208 | test | `expect` is an unconditional panic path: `std::fs::read(&source).expect("read source"),`
R1 | src/install_tests.rs:209 | test | `expect` is an unconditional panic path: `std::fs::read(&target).expect("read target"),`
R1 | src/install_tests.rs:219 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:222 | test | `expect` is an unconditional panic path: `.expect("metadata")`
R1 | src/install_tests.rs:234 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:235 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:243 | test | `expect` is an unconditional panic path: `path_config(Path::new("/x/slugthug/slugaudit")).expect("path config");`
R1 | src/install_tests.rs:252 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:253 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:261 | test | `expect` is an unconditional panic path: `path_config(Path::new("/x/slugthug/slugaudit")).expect("path config");`
R1 | src/install_tests.rs:270 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:271 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:279 | test | `expect` is an unconditional panic path: `path_config(Path::new("/x/slugthug/slugaudit")).expect("path config");`
R1 | src/install_tests.rs:288 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:289 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:297 | test | `expect` is an unconditional panic path: `add_to_path(&bin_dir).expect("first add");`
R1 | src/install_tests.rs:299 | test | `expect` is an unconditional panic path: `let contents = std::fs::read_to_string(&config).expect("read config");`
R1 | src/install_tests.rs:305 | test | `expect` is an unconditional panic path: `add_to_path(&bin_dir).expect("second add");`
R1 | src/install_tests.rs:306 | test | `expect` is an unconditional panic path: `let again = std::fs::read_to_string(&config).expect("read config");`
R1 | src/install_tests.rs:317 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:318 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:319 | test | `expect` is an unconditional panic path: `std::fs::create_dir(temp.path().join(".bashrc")).expect("create config directory");`
R1 | src/install_tests.rs:327 | test | `expect_err` is an unconditional panic path: `let error = add_to_path(&bin_dir).expect_err("a directory cannot be read as config");`
R1 | src/install_tests.rs:337 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:350 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:352 | test | `expect` is an unconditional panic path: `std::fs::write(&home, b"a file where the home should be").expect("plant file");`
R1 | src/install_tests.rs:353 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:358 | test | `panic!` is an unconditional panic path: `panic!("expected Mkdir, got {result:?}");`
R1 | src/install_tests.rs:370 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:372 | test | `expect` is an unconditional panic path: `std::fs::create_dir_all(&target).expect("plant a directory at the target");`
R1 | src/install_tests.rs:373 | test | `expect` is an unconditional panic path: `std::fs::write(target.join("keep"), b"kept").expect("make it non-empty");`
R1 | src/install_tests.rs:374 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:383 | test | `panic!` is an unconditional panic path: `panic!("expected Place, got {result:?}");`
R1 | src/install_tests.rs:391 | test | `expect` is an unconditional panic path: `.expect("list install dir")`
R1 | src/install_tests.rs:405 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:406 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:415 | test | `expect_err` is an unconditional panic path: `let error = path_config(Path::new("/x/slugaudit")).expect_err("non-UTF-8 SHELL fails");`
R1 | src/install_tests.rs:424 | test | `expect` is an unconditional panic path: `let directory = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:426 | test | `expect` is an unconditional panic path: `std::fs::write(&file, b"x").expect("write file");`
R1 | src/install_tests.rs:429 | test | `expect_err` is an unconditional panic path: `.expect_err("a target whose stat fails must not read as a first install");`
R1 | src/install_tests.rs:436 | test | `expect` is an unconditional panic path: `let _guard = TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/install_tests.rs:437 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/install_tests.rs:439 | test | `expect` is an unconditional panic path: `std::fs::write(&file, b"x").expect("write file");`
R1 | src/prompt.rs:101 | test | `expect` is an unconditional panic path: `let first = read_capped_line(&mut input).expect("first line");`
R1 | src/prompt.rs:102 | test | `expect` is an unconditional panic path: `let second = read_capped_line(&mut input).expect("second line");`
R1 | src/prompt.rs:103 | test | `expect` is an unconditional panic path: `let end = read_capped_line(&mut input).expect("end of input");`
R1 | src/prompt.rs:115 | test | `expect` is an unconditional panic path: `let capped = read_capped_line(&mut input).expect("capped line");`
R1 | src/prompt.rs:116 | test | `expect` is an unconditional panic path: `let next = read_capped_line(&mut input).expect("next line");`
R1 | src/prompt.rs:126 | test | `expect` is an unconditional panic path: `let line = read_capped_line(&mut input).expect("lossy, not an error");`
R1 | src/prompt.rs:145 | test | `expect_err` is an unconditional panic path: `.expect_err("a read failure must surface");`
R1 | src/prompt.rs:155 | test | `expect` is an unconditional panic path: `let yes = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"y\n"), "q ").expect("reads");`
R1 | src/prompt.rs:156 | test | `expect` is an unconditional panic path: `let no = ask_yes_no(&mut output, &mut std::io::Cursor::new(b"n\n"), "q ").expect("reads");`
R1 | src/update_tests.rs:21 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:24 | test | `expect` is an unconditional panic path: `std::fs::write(&binary, contents).expect("write fake binary");`
R1 | src/update_tests.rs:32 | test | `expect` is an unconditional panic path: `.expect("run sha256sum");`
R1 | src/update_tests.rs:36 | test | `expect` is an unconditional panic path: `.expect("sha256sum stdout format has digest as first token")`
R1 | src/update_tests.rs:44 | test | `expect` is an unconditional panic path: `.expect("write sha256sums");`
R1 | src/update_tests.rs:46 | test | `expect` is an unconditional panic path: `verify_checksum(&binary, &sums_file).expect("matching checksum must verify");`
R1 | src/update_tests.rs:56 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:61 | test | `expect` is an unconditional panic path: `std::fs::write(&binary, b"#!/bin/sh\necho fake binary\n").expect("write fake binary");`
R1 | src/update_tests.rs:67 | test | `expect` is an unconditional panic path: `.expect("run sha256sum");`
R1 | src/update_tests.rs:71 | test | `expect` is an unconditional panic path: `.expect("sha256sum stdout format has digest as first token")`
R1 | src/update_tests.rs:79 | test | `expect` is an unconditional panic path: `.expect("write sha256sums");`
R1 | src/update_tests.rs:81 | test | `expect` is an unconditional panic path: `verify_checksum(&binary, &sums_file).expect("unique temp name must verify");`
R1 | src/update_tests.rs:89 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:91 | test | `expect` is an unconditional panic path: `std::fs::write(&binary, b"different bytes\n").expect("write fake binary");`
R1 | src/update_tests.rs:98 | test | `expect` is an unconditional panic path: `.expect("write sha256sums");`
R1 | src/update_tests.rs:100 | test | `expect_err` is an unconditional panic path: `let err = verify_checksum(&binary, &sums_file).expect_err("a mismatch must be rejected");`
R1 | src/update_tests.rs:108 | test | `panic!` is an unconditional panic path: `other => panic!("expected Checksum error, got: {other:?}"),`
R1 | src/update_tests.rs:114 | test | `expect` is an unconditional panic path: `let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");`
R1 | src/update_tests.rs:115 | test | `expect` is an unconditional panic path: `let temp = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:117 | test | `expect` is an unconditional panic path: `std::fs::create_dir_all(&slugthug).expect("create dir");`
R1 | src/update_tests.rs:119 | test | `expect` is an unconditional panic path: `std::fs::write(&stable, b"fake installed binary").expect("write fake install");`
R1 | src/update_tests.rs:122 | test | `expect` is an unconditional panic path: `let target = target_binary().expect("target binary resolves");`
R1 | src/update_tests.rs:129 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:131 | test | `expect` is an unconditional panic path: `std::fs::write(&binary, b"content").expect("write binary");`
R1 | src/update_tests.rs:133 | test | `unwrap_err` is an unconditional panic path: `let err = verify_checksum(&binary, &missing_sums).unwrap_err();`
R1 | src/update_tests.rs:139 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:141 | test | `expect` is an unconditional panic path: `std::fs::write(&binary, b"content").expect("write binary");`
R1 | src/update_tests.rs:143 | test | `std::fs::write(&sums_file, "1234567890abcdef  other-asset.tar.gz\n").expect("write sums");` (source has two spaces before the filename).
R1 | src/update_tests.rs:144 | test | `unwrap_err` is an unconditional panic path: `let err = verify_checksum(&binary, &sums_file).unwrap_err();`
R1 | src/update_tests.rs:151 | test | `expect` is an unconditional panic path: `let dir = tempfile::tempdir().expect("temp dir");`
R1 | src/update_tests.rs:154 | test | `std::fs::write(&sums_file, format!("abc  {ASSET}\n")).expect("write sums");` (source has two spaces).
R1 | src/update_tests.rs:155 | test | `unwrap_err` is an unconditional panic path: `let err = verify_checksum(&missing_binary, &sums_file).unwrap_err();`
R1 | src/update_tests.rs:267 | test | `let error = parse_digest("  \n").expect_err("no token is no digest");` (source has two spaces).
R1 | src/update_tests.rs:274 | test | `assert_eq!(parse_digest("ABCDEF  file\n").expect("digest"), "abcdef");` (source has two spaces).
R1 | src/update_fetch_tests.rs:45 | test | `expect` is an unconditional panic path: `compare_versions("1.0.3", "1.0.3").expect("comparable"),`
R1 | src/update_fetch_tests.rs:53 | test | `expect` is an unconditional panic path: `assert_eq!(compare_versions("1.0", "1.0.0").expect("comparable"), Equal);`
R1 | src/update_fetch_tests.rs:54 | test | `expect` is an unconditional panic path: `assert_eq!(compare_versions("1", "1.0.0").expect("comparable"), Equal);`
R1 | src/update_fetch_tests.rs:55 | test | `expect` is an unconditional panic path: `assert_eq!(compare_versions("1.0.0", "1.0").expect("comparable"), Equal);`
R1 | src/update_fetch_tests.rs:56 | test | `expect` is an unconditional panic path: `assert_eq!(compare_versions("1.0", "1.0.1").expect("comparable"), Less);`
R1 | src/update_fetch_tests.rs:58 | test | `expect` is an unconditional panic path: `compare_versions("1.1", "1.0.9").expect("comparable"),`
R1 | src/update_fetch_tests.rs:65 | test | `expect_err` is an unconditional panic path: `let error = compare_versions("1.0.0-rc1", "1.0.0").expect_err("pre-release is not numeric");`
R1 | src/update_fetch_tests.rs:86 | test | `expect` is an unconditional panic path: `let release = LatestRelease::from_json(&json).expect("parses");`
R1 | src/ignore_rules_tests.rs:14 | test | `expect` is an unconditional panic path: `tempfile::tempdir().expect("project dir")`
R1 | src/ignore_rules_tests.rs:27 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".gitignore"), "target/\n").expect("gitignore");`
R1 | src/ignore_rules_tests.rs:28 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("target")).expect("target dir");`
R1 | src/ignore_rules_tests.rs:29 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").expect("target file");`
R1 | src/ignore_rules_tests.rs:30 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("src")).expect("src dir");`
R1 | src/ignore_rules_tests.rs:31 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").expect("lib file");`
R1 | src/ignore_rules_tests.rs:45 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("sub")).expect("sub dir");`
R1 | src/ignore_rules_tests.rs:46 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("other")).expect("other dir");`
R1 | src/ignore_rules_tests.rs:47 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/.gitignore"), "*.log\n").expect("nested gitignore");`
R1 | src/ignore_rules_tests.rs:48 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/debug.log"), "x\n").expect("sub log");`
R1 | src/ignore_rules_tests.rs:49 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("debug.log"), "x\n").expect("root log");`
R1 | src/ignore_rules_tests.rs:50 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("other/debug.log"), "x\n").expect("other log");`
R1 | src/ignore_rules_tests.rs:63 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".gitignore"), "*.log\n!keep.log\n").expect("gitignore");`
R1 | src/ignore_rules_tests.rs:64 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("keep.log"), "x\n").expect("keep file");`
R1 | src/ignore_rules_tests.rs:65 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("debug.log"), "x\n").expect("debug file");`
R1 | src/ignore_rules_tests.rs:83 | test | `expect` is an unconditional panic path: `.expect("gitignore");`
R1 | src/ignore_rules_tests.rs:84 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("build")).expect("build dir");`
R1 | src/ignore_rules_tests.rs:85 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("build/keep.rs"), "fn keep() {}\n").expect("keep file");`
R1 | src/ignore_rules_tests.rs:97 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".gitignore"), "*.log\n").expect("gitignore");`
R1 | src/ignore_rules_tests.rs:98 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".ignore"), "!keep.log\n").expect("dot ignore");`
R1 | src/ignore_rules_tests.rs:99 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("keep.log"), "x\n").expect("keep file");`
R1 | src/ignore_rules_tests.rs:112 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join(".planning/slugaudit")).expect("data dir");`
R1 | src/ignore_rules_tests.rs:113 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("src")).expect("src dir");`
R1 | src/ignore_rules_tests.rs:114 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".planning/slugaudit/project.db"), "db").expect("db");`
R1 | src/ignore_rules_tests.rs:115 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("src/lib.rs~"), "backup").expect("swap file");`
R1 | src/ignore_rules_tests.rs:116 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("notes.tmp"), "tmp").expect("tmp file");`
R1 | src/ignore_rules_tests.rs:132 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("sub")).expect("sub dir");`
R1 | src/ignore_rules_tests.rs:133 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".ignore"), "!keep.log\n").expect("root dot ignore");`
R1 | src/ignore_rules_tests.rs:134 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/.gitignore"), "*.log\n").expect("nested gitignore");`
R1 | src/ignore_rules_tests.rs:135 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/keep.log"), "x\n").expect("keep file");`
R1 | src/ignore_rules_tests.rs:136 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/debug.log"), "x\n").expect("debug file");`
R1 | src/ignore_rules_tests.rs:152 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("sub")).expect("sub dir");`
R1 | src/ignore_rules_tests.rs:153 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".ignore"), "!keep.log\n").expect("root dot ignore");`
R1 | src/ignore_rules_tests.rs:154 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/.gitignore"), "*.log\n").expect("nested gitignore");`
R1 | src/ignore_rules_tests.rs:155 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/keep.log"), "x\n").expect("keep file");`
R1 | src/ignore_rules_tests.rs:156 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/debug.log"), "x\n").expect("debug file");`
R1 | src/ignore_rules_tests.rs:158 | test | `expect` is an unconditional panic path: `let (files, _skipped) = discover(project.path()).expect("discover");`
R1 | src/ignore_rules_tests.rs:189 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".gitignore"), "target/\n*.log\n").expect("gitignore");`
R1 | src/ignore_rules_tests.rs:190 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("target")).expect("target dir");`
R1 | src/ignore_rules_tests.rs:191 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").expect("target file");`
R1 | src/ignore_rules_tests.rs:192 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("sub")).expect("sub dir");`
R1 | src/ignore_rules_tests.rs:193 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/.gitignore"), "secret.rs\n").expect("nested gitignore");`
R1 | src/ignore_rules_tests.rs:194 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/secret.rs"), "fn s() {}\n").expect("secret file");`
R1 | src/ignore_rules_tests.rs:195 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("sub/open.rs"), "fn o() {}\n").expect("open file");`
R1 | src/ignore_rules_tests.rs:196 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("debug.log"), "x\n").expect("log file");`
R1 | src/ignore_rules_tests.rs:197 | test | `expect` is an unconditional panic path: `fs::create_dir_all(project.path().join("src")).expect("src dir");`
R1 | src/ignore_rules_tests.rs:198 | test | `expect` is an unconditional panic path: `fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").expect("lib file");`
R1 | src/ignore_rules_tests.rs:200 | test | `expect` is an unconditional panic path: `let (files, _skipped) = discover(project.path()).expect("discover");`
R1 | src/ignore_rules_tests.rs:269 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".gitignore"), "*.tmp\n").expect("gitignore");`
R1 | src/ignore_rules_tests.rs:272 | test | `expect` is an unconditional panic path: `fs::write(project.path().join(".git"), "gitdir: elsewhere\n").expect("git file");`
R2 | src/install_tests.rs:392 | test | `filter_map(Result::ok)` silently drops `read_dir` entry errors while collecting leftover staged files, so cleanup failures cannot be observed.
R3 | src/connect.rs:2 | prod | `#![allow(clippy::print_stdout)]` lint waiver at module scope.
R3 | src/connect_exec.rs:2 | prod | `#![allow(clippy::print_stdout)]` lint waiver at module scope.
R3 | src/install.rs:2 | prod | `#![allow(clippy::print_stdout)]` lint waiver at module scope.
R3 | src/menu.rs:4 | prod | `#![allow(clippy::print_stdout)]` lint waiver at module scope.
R3 | src/menu_render.rs:2 | prod | `#![allow(clippy::print_stdout)]` lint waiver at module scope.
R3 | src/update.rs:30 | prod | `#![allow(clippy::print_stdout)]` lint waiver at module scope.
R5 | src/connect.rs:54 | prod | `ConnectError::EndOfInput` from `read_answer`; no test asserts it (the interactive prompts have no test).
R5 | src/connect.rs:58 | prod | `ConnectError::BinaryPath` from `running_binary`; no test produces it.
R5 | src/connect.rs:59 | prod | `ConnectError::Io` from the `prefer_slugthug_binary` stat; the only test (connect_registry_tests.rs:516) asserts `is_err()`, not the variant.
R5 | src/connect.rs:105 | prod | `ConnectError::Io` from `stdout().flush()?` in `run_connect_interactive` (same arm at 118 and 146); no test drives this path.
R5 | src/connect_cli.rs:108 | prod | `ConnectError::AgentCommand` from the `run_add` spawn failure; the only AgentCommand test (connect_tests.rs:513) exercises `run_remove`.
R5 | src/connect_exec.rs:180 | prod | `ConnectError::HomeUnavailable` from `write_agent_config`; no test asserts it.
R5 | src/connect_exec.rs:215 | prod | `ConnectError::HomeUnavailable` from `write_atomic` (path with no parent); no test asserts it.
R5 | src/connect_exec.rs:218 | prod | `ConnectError::Io` from the config staging write (also rename at 225, create_dir_all at 247); no test asserts the variant.
R5 | src/install.rs:107 | prod | `InstallError::CurrentExe`; no test produces a failing `running_binary`.
R5 | src/install.rs:117 | prod | `InstallError::Clock`; no test asserts it.
R5 | src/install.rs:122 | prod | `InstallError::Copy` from the staged-copy failure; no test asserts it.
R5 | src/menu.rs:23 | prod | `run_menu` propagates the stdout-flush error as `Box<dyn Error>` (also the stdin read at 24); tests/menu_tests.rs drives the menu end-to-end but no test asserts either failure path.
R5 | src/update.rs:96 | prod | `UpdateError::Target`; only constructed for a Display test (update_tests.rs:170), no failure path asserts it.
R5 | src/update.rs:103 | prod | `UpdateError::Stage` for a target with no parent; no test asserts it.
R5 | src/update.rs:126 | prod | `UpdateError::Stage` mapped from `PlaceError`; no test asserts it (the Place test covers `InstallError::Place`).
R5 | src/update.rs:168 | prod | `UpdateError::ToolSpawn` for a missing `sha256sum`; no test simulates it.
R5 | src/update.rs:227 | prod | `UpdateError::UnsupportedPlatform` from `run_update`; only Display (update_tests.rs:235) and `platform_supported` are tested, not the returned variant.
R5 | src/update_fetch.rs:51 | prod | `UpdateError::NoTag` from `from_json`; only Display-tested (update_tests.rs:184).
R5 | src/update_fetch.rs:78 | prod | `UpdateError::Json` from `fetch_latest_release`; only Display-tested (update_tests.rs:180).
R5 | src/update_fetch.rs:91 | prod | `UpdateError::ToolSpawn` for a missing `curl` in `curl_get`; no test.
R5 | src/update_fetch.rs:96 | prod | `UpdateError::Network` from a failing `curl` in `curl_get`; no test.
R5 | src/update_fetch.rs:119 | prod | `UpdateError::ToolSpawn` for a missing `curl` in `download`; no test.
R5 | src/update_fetch.rs:125 | prod | `UpdateError::Network` from a failing `curl` in `download`; no test.
R6 | src/connect.rs:81 | prod | Prints "Done." after `disconnect_agent`, which returns `Ok` without touching anything when `config_home()` is `None` (connect_exec.rs:272) and for entries with neither a writable section nor a CLI (goose, crush): `slugaudit disconnect <agent>` reports success for a no-op.
R6 | src/connect_cli.rs:114 | prod | Client stderr is echoed only when non-empty, but `ConnectError::AddFailed` tells the user to "see the output above"; a client that exits non-zero with empty stderr leaves that instruction false.
R6 | src/connect_guidance.rs:83 | prod | Failure advice says "install `{cli}` and re-run" even when the CLI is on PATH and its `mcp add` rejected the call (hint `Some(attempted)` from connect_exec.rs:155), describing an install that already happened.
R8 | src/connect_agents.rs:43 | prod | Public variant `ConfigShape::McpServersCommandArgs` has no reference outside `connect_agents.rs` (other modules match through methods only).
R8 | src/connect_agents.rs:74 | prod | Public variant `ConfigShape::Unknown` has no reference outside `connect_agents.rs`.
R8 | src/connect_agents.rs:143 | prod | Public variant `ConfigPath::PerOs` has no reference outside `connect_agents.rs`.
R8 | src/connect_agents.rs:144 | prod | Public field `ConfigPath::PerOs::macos` has no doc comment.
R8 | src/connect_agents.rs:145 | prod | Public field `ConfigPath::PerOs::windows` has no doc comment.
R8 | src/connect_agents.rs:146 | prod | Public field `ConfigPath::PerOs::linux` has no doc comment.
R8 | src/connect_agents.rs:149 | prod | Public variant `ConfigPath::Unverified` has no reference outside `connect_agents.rs`.
R8 | src/connect_agents.rs:253 | prod | Public field `AgentDef::id` has no doc comment.
R8 | src/connect_agents.rs:254 | prod | Public field `AgentDef::display_name` has no doc comment.
R8 | src/connect_agents.rs:255 | prod | Public field `AgentDef::strategy` has no doc comment.
R8 | src/connect_agents.rs:263 | prod | Public field `AgentDef::config` has no doc comment.
R8 | src/connect_agents.rs:264 | prod | Public field `AgentDef::shape` has no doc comment.
R8 | src/connect_agents.rs:265 | prod | Public field `AgentDef::dialect` has no doc comment.
R8 | src/connect_agents.rs:266 | prod | Public field `AgentDef::evidence` has no doc comment.
R8 | src/install.rs:45 | prod | Public variant `InstallError::CurrentExe` has no doc comment.
R8 | src/install.rs:47 | prod | Public variant `InstallError::NoHome` has no doc comment.
R8 | src/install.rs:49 | prod | Public variant `InstallError::Mkdir` and its fields `path`, `inner` have no doc comments.
R8 | src/install.rs:51 | prod | Public variant `InstallError::Copy` and its fields `path`, `inner` have no doc comments.
R8 | src/install.rs:53 | prod | Public variant `InstallError::Clock` has no doc comment.
R8 | src/install.rs:55 | prod | Public variant `InstallError::ShellNotUnicode` has no doc comment.
R8 | src/install.rs:57 | prod | Public variant `InstallError::Place` has no doc comment.
R8 | src/install.rs:59 | prod | Public variant `InstallError::Io` has no doc comment.
R8 | src/update.rs:50 | prod | Public variant `UpdateError::ToolSpawn` has no doc comment.
R8 | src/update.rs:51 | prod | Public field `UpdateError::ToolSpawn::tool` has no doc comment.
R8 | src/update.rs:53 | prod | Public field `UpdateError::ToolSpawn::source` has no doc comment.
R8 | src/update.rs:56 | prod | Public variant `UpdateError::Target` has no doc comment.
R8 | src/update.rs:58 | prod | Public variant `UpdateError::Network` has no doc comment.
R8 | src/update.rs:60 | prod | Public variant `UpdateError::Json` has no doc comment.
R8 | src/update.rs:62 | prod | Public variant `UpdateError::NoTag` has no doc comment.
R8 | src/update.rs:64 | prod | Public variant `UpdateError::Checksum` has no doc comment.
R8 | src/update.rs:70 | prod | Public variant `UpdateError::UnsupportedPlatform` has no doc comment.
R8 | src/update.rs:71 | prod | Public field `UpdateError::UnsupportedPlatform::os` has no doc comment.
R8 | src/update.rs:72 | prod | Public field `UpdateError::UnsupportedPlatform::arch` has no doc comment.
R8 | src/update.rs:75 | prod | Public variant `UpdateError::Io` has no doc comment.
R8 | src/update.rs:77 | prod | Public variant `UpdateError::Download` has no doc comment.
R8 | src/update.rs:78 | prod | Public field `UpdateError::Download::action` has no doc comment.
R8 | src/update.rs:79 | prod | Public field `UpdateError::Download::path` has no doc comment.
R8 | src/update.rs:81 | prod | Public field `UpdateError::Download::source` has no doc comment.
R8 | src/update.rs:84 | prod | Public variant `UpdateError::Stage` has no doc comment.
R8 | src/update.rs:85 | prod | Public field `UpdateError::Stage::action` has no doc comment.
R8 | src/update.rs:86 | prod | Public field `UpdateError::Stage::path` has no doc comment.
R8 | src/update.rs:88 | prod | Public field `UpdateError::Stage::source` has no doc comment.
R10 | src/connect_tests.rs:56-79, src/connect_tests.rs:81-102 | test | Two connect/disconnect tests copy the same ~18-line sequence with only the agent id and config path substituted; a shared helper would keep the assertions single-sourced.
R10 | src/install_tests.rs:232-248, src/install_tests.rs:250-266 | test | The bash and fish `path_config` tests are the same ~14-line env-sandbox and assertion block with two literals changed.
R10 | src/update_tests.rs:19-47, src/update_tests.rs:53-82 | test | Two checksum tests copy the same ~20-line setup (tempdir, sha256sum digest extraction, SHA256SUMS write, verify); only the fixture filename differs.

## Chunk 3

R1 | src/evidence/mod.rs:125 | test | `panic!` fallback in the `NormalizerUnavailable` match arm.
R1 | src/evidence/mod.rs:140 | test | `.expect("whole code point")` on the `node_text` result.
R1 | src/evidence/normalize_grammar_tests.rs:28 | test | `.expect("extract")` in the `extract_items` helper.
R1 | src/evidence/normalize_tests.rs:8 | test | `.expect("extract")` in the `extract_items` helper.
R1 | src/evidence/normalize_tests.rs:32 | test | `.expect("a structure item")` on a searched item.
R1 | src/evidence/normalize_tests.rs:67 | test | `.expect("fits")` on `usize::try_from(MAX_WALK_DEPTH)`.
R1 | src/evidence/normalize_tests.rs:71 | test | `.expect(...)` on `extract("python", ...)`.
R1 | src/evidence/normalize_tests.rs:95 | test | `.expect(...)` on `extract("rust", ...)` for the macro-bound case.
R1 | src/evidence/normalize_tests.rs:105 | test | `.expect("extract")` on `extract("rust", ...)`.
R1 | src/evidence/normalize_tests.rs:126 | test | `.expect("extract")` on the multi-megabyte single-line source.
R1 | src/evidence/normalize_tests.rs:164 | test | `panic!("unknown language should fail")` in the error match.
R1 | src/evidence/normalize_tests.rs:190 | test | `.expect("name is a string")` on a payload field.
R1 | src/evidence/normalize_tests.rs:212 | test | `.expect("name is a string")` on a payload field.
R1 | src/evidence/normalize_tests.rs:240 | test | `.expect("name is a string")` on a payload field.
R1 | src/evidence/normalize_tests.rs:263 | test | `.expect("name is a string")` on a payload field.
R1 | src/evidence/normalize_tests.rs:282 | test | `.expect("name is a string")` on a payload field.
R1 | src/evidence/normalize_tests.rs:354 | test | `.expect("extract")` inside the spawned extraction thread.
R1 | src/evidence/normalize_tests.rs:355 | test | `.expect("spawn small-stack thread")` on thread spawn.
R1 | src/evidence/normalize_tests.rs:357 | test | `.expect("extraction thread panicked")` on `join`.
R1 | src/evidence/normalize_tests.rs:462 | test | `.expect("extract")` inside the 2 MiB production-size thread.
R1 | src/evidence/normalize_tests.rs:463 | test | `.expect("spawn production-size thread")` on thread spawn.
R1 | src/evidence/normalize_tests.rs:465 | test | `.expect("extraction thread panicked")` on `join`.
R1 | src/evidence/sql.rs:118 | test | `.expect("valid span")` when building the test fixture span.
R1 | src/evidence/sql.rs:142 | test | `.expect("valid span")` when building the u64::MAX span fixture.
R1 | src/evidence/calls_tests.rs:20 | test | `.expect("callee_name as string")` on a payload field.
R1 | src/evidence/calls_tests.rs:30 | test | `.expect("expected run_call helper")` on the searched call.
R1 | src/evidence/calls_tests.rs:37 | test | `.expect("expected macro_call println")` on the searched call.
R1 | src/graph/reference_tests.rs:9 | test | `.expect("reference should extract")` on a python relative form.
R1 | src/graph/reference_tests.rs:16 | test | `.expect("reference should extract")` on a python relative form.
R1 | src/graph/reference_tests.rs:23 | test | `.expect("reference should extract")` on a python dotted form.
R1 | src/graph/reference_tests.rs:30 | test | `.expect("reference should extract")` on `import os`.
R1 | src/graph/reference_tests.rs:37 | test | `.expect("reference should extract")` on `import numpy as np`.
R1 | src/graph/reference_tests.rs:44 | test | `.expect("reference should extract")` on a from-import.
R1 | src/graph/reference_tests.rs:56 | test | `.expect("reference should extract")` on a JS relative path.
R1 | src/graph/reference_tests.rs:63 | test | `.expect("reference should extract")` on a JS parent path.
R1 | src/graph/reference_tests.rs:70 | test | `.expect("reference should extract")` on a bare JS package.
R1 | src/graph/reference_tests.rs:77 | test | `.expect("reference should extract")` on a TS type import.
R1 | src/graph/reference_tests.rs:84 | test | `.expect("reference should extract")` on a double-quoted JS import.
R1 | src/graph/reference_tests.rs:96 | test | `.expect("reference should extract")` on a rust `use crate::` path.
R1 | src/graph/reference_tests.rs:103 | test | `.expect("reference should extract")` on a rust std path.
R1 | src/graph/reference_tests.rs:110 | test | `.expect("reference should extract")` on `use super::thing`.
R1 | src/graph/reference_tests.rs:117 | test | `.expect("reference should extract")` on `use self::inner`.
R1 | src/graph/reference_tests.rs:124 | test | `.expect("reference should extract")` on `pub use`.
R1 | src/graph/resolver/gate_tests.rs:36 | test | `.expect("import from_util must not be dropped")` on extraction.
R1 | src/graph/resolver/gate_tests.rs:49 | test | `.expect("import foo_bar.from_baz must not be dropped")` on extraction.
R1 | src/graph/resolver/gate_tests.rs:68 | test | `.expect("extract succeeds")` on extraction.
R1 | src/graph/resolver/gate_tests.rs:99 | test | `.expect("valid JS import extracts cleanly")` on extraction.
R1 | src/graph/resolver/gate_tests.rs:113 | test | `.expect("valid path with the word 'from' inside it")` on extraction.
R1 | src/graph/resolver/proptest.rs:117 | test | `.expect("extract_reference returned None")` in the proptest body.
R1 | src/model/limits_tests.rs:23 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:35 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:52 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:61 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:79 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:93 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:113 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:143 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:162 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/limits_tests.rs:181 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/profile_tests.rs:30 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/profile_tests.rs:46 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/profile_tests.rs:64 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/profile_tests.rs:85 | test | `.expect("env lock")` on the `TEST_ENV_LOCK` mutex.
R1 | src/model/span_tests.rs:6 | test | `.expect("valid span")` on `Span::new`.
R1 | src/model/span_tests.rs:102 | test | `.expect("fits")` on `usize::try_from(state >> 33)`.
R1 | src/model/span_tests.rs:134 | test | `.expect("deserialize bypasses Span::new")` on serde parsing.
R1 | src/model/system_ram.rs:127 | test | `.expect("parse succeeds")` on `parse_proc_meminfo`.
R1 | src/model/system_ram.rs:135 | test | `.expect("parse succeeds")` on `parse_proc_meminfo`.
R5 | src/model/parser.rs:135 | prod | `ParserRun::validate`'s second error arm `"successful parsing cannot have unavailable extraction"` has no test asserting it; only the first arm is covered (parser.rs:169-179) and no test names this string (grep of src/, tests/, benches/ finds it only at the error site).
R8 | src/evidence/sql.rs:7 | prod | Public field `EvidenceRow::key` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:8 | prod | Public field `EvidenceRow::kind` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:9 | prod | Public field `EvidenceRow::origin` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:10 | prod | Public field `EvidenceRow::span_availability` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:11 | prod | Public field `EvidenceRow::start_byte` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:12 | prod | Public field `EvidenceRow::end_byte` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:13 | prod | Public field `EvidenceRow::start_line` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:14 | prod | Public field `EvidenceRow::start_column` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:15 | prod | Public field `EvidenceRow::end_line` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:16 | prod | Public field `EvidenceRow::end_column` lacks a doc comment (missing_docs).
R8 | src/evidence/sql.rs:17 | prod | Public field `EvidenceRow::payload` lacks a doc comment (missing_docs).
R8 | src/graph/mod.rs:25 | prod | `pub fn is_supported_language` has no reference outside `graph`: the only call is the `resolver::` pub(crate) twin used inside this module; nothing in `src/`, `tests/`, or `benches/` calls the public wrapper.
R8 | src/graph/mod.rs:33 | prod | Public field `ResolvedEdge::raw_import_text` lacks a doc comment (missing_docs).
R8 | src/graph/mod.rs:34 | prod | Public field `ResolvedEdge::resolution_kind` lacks a doc comment (missing_docs).
R8 | src/graph/mod.rs:35 | prod | Public field `ResolvedEdge::confidence` lacks a doc comment (missing_docs).
R8 | src/graph/mod.rs:36 | prod | Public field `ResolvedEdge::to_relative_path` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:9 | prod | Public variant `SpanAvailability::Present` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:10 | prod | Public variant `SpanAvailability::PackOmitted` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:11 | prod | Public variant `SpanAvailability::NormalizerUnavailable` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:12 | prod | Public variant `SpanAvailability::DerivedEvidence` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:25 | prod | Public variant `EvidenceKind::Structure` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:26 | prod | Public variant `EvidenceKind::Import` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:27 | prod | Public variant `EvidenceKind::Export` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:28 | prod | Public variant `EvidenceKind::Comment` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:29 | prod | Public variant `EvidenceKind::Docstring` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:30 | prod | Public variant `EvidenceKind::Symbol` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:31 | prod | Public variant `EvidenceKind::Diagnostic` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:32 | prod | Public variant `EvidenceKind::Call` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:39 | prod | Public field `EvidenceItem::key` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:40 | prod | Public field `EvidenceItem::kind` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:41 | prod | Public field `EvidenceItem::origin` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:42 | prod | Public field `EvidenceItem::span` lacks a doc comment (missing_docs).
R8 | src/model/evidence.rs:43 | prod | Public field `EvidenceItem::payload` lacks a doc comment (missing_docs).
R8 | src/model/limits.rs:8 | prod | Public field `EvidenceLimits::max_items_per_file` lacks a doc comment (missing_docs).
R8 | src/model/limits.rs:9 | prod | Public field `EvidenceLimits::max_payload_bytes_per_item` lacks a doc comment (missing_docs).
R8 | src/model/limits.rs:10 | prod | Public field `EvidenceLimits::max_payload_bytes_per_file` lacks a doc comment (missing_docs).
R8 | src/model/limits.rs:130 | prod | Public field `ResourceLimits::evidence` lacks a doc comment (missing_docs).
R8 | src/model/limits.rs:156 | prod | `pub fn ResourceLimits::from_env` has no reference outside `model::limits`: it is called only by `process_limits` (same module) and `limits_tests.rs`; no other `src/`, `tests/`, or `benches/` caller exists.
R8 | src/model/parser.rs:11 | prod | Public struct-variant field `ParserAvailability::LoadFailed::reason` lacks a doc comment (missing_docs).
R8 | src/model/parser.rs:36 | prod | Public struct-variant field `ParseOutcome::SyntaxErrors::count` lacks a doc comment (missing_docs).
R8 | src/model/parser.rs:38 | prod | Public struct-variant field `ParseOutcome::Failed::reason` lacks a doc comment (missing_docs).
R8 | src/model/source.rs:16 | prod | `pub struct FileMetadata` has no reference outside `model::source` (only its own use as a `SourceSnapshot` field and the `model/mod.rs` re-export); no `src/`, `tests/`, or `benches/` use.
R8 | src/model/source.rs:25 | prod | `pub struct LanguageSelection` has no reference outside `model::source` (only its own `SourceSnapshot` field and the re-export); no `src/`, `tests/`, or `benches/` use.
R8 | src/model/source.rs:34 | prod | `pub struct SourceSnapshot` has no reference outside `model::source`; only the `model/mod.rs` re-export names it.
R8 | src/model/span.rs:45 | prod | `pub fn char_column` has no reference outside `model::span`: call sites are only the function's own legacy tests; everything else migrated to `LineIndex`.
R8 | src/model/span.rs:112 | prod | `pub fn LineIndex::line` has no caller outside `model::span`/`span_tests.rs`; external users call `column`/`position`.
R8 | src/model/span.rs:223 | prod | `pub fn Span::byte_len` has no reference outside `model::span`; the only calls are in `span_tests.rs`.
R10 | src/evidence/normalize.rs:320-335 | prod | Near-verbatim 16-line `node_span` clone: src/evidence/normalize.rs:320-335 and src/evidence/generic_imports.rs:176-191 differ only in type paths and the `"binding"`/`"generic import"` context string; the span construction should be shared beside `span_unavailable`.
R10 | src/evidence/normalize.rs:320-335 | prod | Same span-building logic again at src/evidence/calls.rs:213-226 (14 lines; the position construction and span fallback are duplicated, differing in the local byte bindings, the column expressions, and the final `match` vs `map_or_else`) — a third copy of the start/end position construction.

## Chunk 4

R1 | src/store/connection_tests.rs:14 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:15 | test | test-scope .expect("open")
R1 | src/store/connection_tests.rs:18 | test | test-scope .expect("read pragma")
R1 | src/store/connection_tests.rs:24 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:26 | test | test-scope .expect("create database")
R1 | src/store/connection_tests.rs:28 | test | test-scope .expect("open read-only")
R1 | src/store/connection_tests.rs:39 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:57 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:60 | test | test-scope .expect("create project database")
R1 | src/store/connection_tests.rs:63 | test | test-scope .expect("create foreign db")
R1 | src/store/connection_tests.rs:69 | test | test-scope .expect("seed foreign db")
R1 | src/store/connection_tests.rs:75 | test | test-scope .expect("open read-only")
R1 | src/store/connection_tests.rs:79 | test | test-scope .expect("open read-write")
R1 | src/store/connection_tests.rs:103 | test | test-scope .expect("{label} connection still serves ordinary reads")
R1 | src/store/connection_tests.rs:115 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:117 | test | test-scope .expect("write garbage")
R1 | src/store/connection_tests.rs:122 | test | test-scope .expect("protect garbage fixture")
R1 | src/store/connection_tests.rs:135 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:138 | test | test-scope .expect("create symlink")
R1 | src/store/connection_tests.rs:158 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:160 | test | test-scope .expect("create database")
R1 | src/store/connection_tests.rs:161 | test | test-scope .expect("stat")
R1 | src/store/connection_tests.rs:177 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:179 | test | test-scope .expect("create database")
R1 | src/store/connection_tests.rs:182 | test | test-scope .expect("a write materialises the write-ahead log")
R1 | src/store/connection_tests.rs:187 | test | test-scope panic!(..)
R1 | src/store/connection_tests.rs:208 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:218 | test | test-scope .expect("simulate a concurrent creator")
R1 | src/store/connection_tests.rs:222 | test | test-scope .expect("widen simulated concurrent creator")
R1 | src/store/connection_tests.rs:229 | test | test-scope .expect("stat")
R1 | src/store/connection_tests.rs:246 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:250 | test | test-scope .expect("first open on a real file succeeds")
R1 | src/store/connection_tests.rs:255 | test | test-scope .expect("symlink metadata readable")
R1 | src/store/connection_tests.rs:263 | test | test-scope .expect("remove the real file")
R1 | src/store/connection_tests.rs:264 | test | test-scope .expect("replace it with a symlink")
R1 | src/store/connection_tests.rs:285 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:288 | test | test-scope .expect("create symlink")
R1 | src/store/connection_tests.rs:314 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:327 | test | test-scope .expect("create with wide mode")
R1 | src/store/connection_tests.rs:331 | test | test-scope .expect("widen permissions fixture")
R1 | src/store/connection_tests.rs:384 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:421 | test | test-scope .expect("the derived EvidenceKind schema serializes as JSON")
R1 | src/store/connection_tests.rs:425 | test | test-scope .expect("the EvidenceKind schema lists its variants as a flat enum of ...")
R1 | src/store/connection_tests.rs:431 | test | test-scope .expect("every variant name in the schema deserializes into an Evidenc...")
R1 | src/store/connection_tests.rs:446 | test | test-scope .expect("read the evidence table DDL")
R1 | src/store/connection_tests.rs:449 | test | test-scope .expect("evidence.kind must carry a CHECK (kind IN (...))")
R1 | src/store/connection_tests.rs:452 | test | test-scope .expect("the kind CHECK's closing paren")
R1 | src/store/connection_tests.rs:471 | test | test-scope .expect("insert the parent revision")
R1 | src/store/connection_tests.rs:481 | test | test-scope .expect("insert the parent file")
R1 | src/store/connection_tests.rs:491 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:492 | test | test-scope .expect("open")
R1 | src/store/connection_tests.rs:514 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:515 | test | test-scope .expect("open")
R1 | src/store/connection_tests.rs:524 | test | test-scope .expect("read the size")
R1 | src/store/connection_tests.rs:534 | test | test-scope .expect("a skipped row may carry an unknown size")
R1 | src/store/connection_tests.rs:546 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:547 | test | test-scope .expect("open")
R1 | src/store/connection_tests.rs:597 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:599 | test | test-scope .expect("real dir")
R1 | src/store/connection_tests.rs:601 | test | test-scope .expect("dir symlink")
R1 | src/store/connection_tests.rs:604 | test | test-scope .expect("read-write open through a symlinked parent")
R1 | src/store/connection_tests.rs:619 | test | test-scope .expect("temp dir")
R1 | src/store/connection_tests.rs:621 | test | test-scope .expect("create project database")
R1 | src/store/connection_tests.rs:626 | test | test-scope .expect("open read-only")
R1 | src/store/connection_tests.rs:630 | test | test-scope .expect("open read-write")
R1 | src/store/connection_tests.rs:639 | test | test-scope panic!(..)
R1 | src/store/connection_tests.rs:665 | test | test-scope .expect("in-memory db")
R1 | src/store/migrations_tests.rs:6 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:7 | test | test-scope .expect("open")
R1 | src/store/migrations_tests.rs:15 | test | test-scope .expect("query sqlite_master")
R1 | src/store/migrations_tests.rs:20 | test | test-scope .expect("user_version")
R1 | src/store/migrations_tests.rs:26 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:28 | test | test-scope .expect("first open")
R1 | src/store/migrations_tests.rs:29 | test | test-scope .expect("second open")
R1 | src/store/migrations_tests.rs:33 | test | test-scope .expect("read version")
R1 | src/store/migrations_tests.rs:39 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:40 | test | test-scope .expect("open")
R1 | src/store/migrations_tests.rs:49 | test | test-scope .expect("query sqlite_master")
R1 | src/store/migrations_tests.rs:55 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:57 | test | test-scope .expect("open database")
R1 | src/store/migrations_tests.rs:60 | test | test-scope .expect("bump version")
R1 | src/store/migrations_tests.rs:75 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:77 | test | test-scope .expect("open database")
R1 | src/store/migrations_tests.rs:80 | test | test-scope .expect("rewind version to v1")
R1 | src/store/migrations_tests.rs:97 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:99 | test | test-scope .expect("open database")
R1 | src/store/migrations_tests.rs:102 | test | test-scope .expect("rewind version to v2")
R1 | src/store/migrations_tests.rs:119 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:121 | test | test-scope .expect("write garbage")
R1 | src/store/migrations_tests.rs:122 | test | test-scope .expect("open handle")
R1 | src/store/migrations_tests.rs:135 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:137 | test | test-scope .expect("create empty database")
R1 | src/store/migrations_tests.rs:140 | test | test-scope .expect("open read-only")
R1 | src/store/migrations_tests.rs:156 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:158 | test | test-scope .expect("open database")
R1 | src/store/migrations_tests.rs:159 | test | test-scope .expect("second pass")
R1 | src/store/migrations_tests.rs:160 | test | test-scope .expect("third pass")
R1 | src/store/migrations_tests.rs:163 | test | test-scope .expect("user_version")
R1 | src/store/migrations_tests.rs:172 | test | test-scope .expect("temp dir")
R1 | src/store/migrations_tests.rs:173 | test | test-scope .expect("open")
R1 | src/store/migrations_tests.rs:176 | test | test-scope .expect("set user_version")
R1 | src/store/netfs_tests.rs:11 | test | test-scope .expect("temp dir")
R1 | src/store/netfs_tests.rs:204 | test | test-scope .expect("temp dir")
R1 | src/store/sql_functions_tests.rs:4 | test | test-scope .expect("in-memory database")
R1 | src/store/sql_functions_tests.rs:5 | test | test-scope .expect("register line functions")
R1 | src/store/sql_functions_tests.rs:14 | test | test-scope .expect("line_range")
R1 | src/store/sql_functions_tests.rs:20 | test | test-scope .expect("line_count")
R1 | src/store/sql_functions_tests.rs:103 | test | test-scope .expect("null calls succeed")
R1 | src/store/sql_functions_tests.rs:133 | test | test-scope .expect("a generated column accepts only deterministic functions")
R1 | src/store/sql_functions_tests.rs:136 | test | test-scope .expect("an index expression accepts only deterministic functions")
R1 | src/store/test_capture.rs:27 | test | test-scope .expect("capture buffer poisoned")
R1 | src/store/test_capture.rs:110 | test | test-scope .expect("capture buffer poisoned")
R1 | src/watch/mod.rs:59 | test | test-scope .expect("walk hook lock")
R1 | src/watch/mod.rs:65 | test | test-scope .expect("walk hook lock")
R1 | src/watch/manager_event_tests.rs:16 | test | test-scope .expect("canonical temp dir")
R1 | src/watch/manager_event_tests.rs:17 | test | test-scope .expect("project dir")
R1 | src/watch/manager_event_tests.rs:19 | test | test-scope .expect("activation dir")
R1 | src/watch/manager_event_tests.rs:116 | test | test-scope .expect("outside dir")
R1 | src/watch/manager_event_tests.rs:160 | test | test-scope .expect("gitignore")
R1 | src/watch/manager_event_tests.rs:161 | test | test-scope .expect("target dir")
R1 | src/watch/manager_event_tests.rs:162 | test | test-scope .expect("target file")
R1 | src/watch/manager_event_tests.rs:163 | test | test-scope .expect("lib file")
R1 | src/watch/manager_event_tests.rs:190 | test | test-scope .expect("src dir")
R1 | src/watch/manager_event_tests.rs:191 | test | test-scope .expect("lib file")
R1 | src/watch/manager_event_tests.rs:211 | test | test-scope .expect("gitignore")
R1 | src/watch/manager_event_tests.rs:243 | test | test-scope .expect("remove project root")
R1 | src/watch/manager_event_tests.rs:262 | test | test-scope .expect("hold the manager lock")
R1 | src/watch/manager_event_tests.rs:285 | test | test-scope .expect("hold the manager lock")
R1 | src/watch/manager_event_tests.rs:305 | test | test-scope .expect("hold the manager lock")
R1 | src/watch/manager_event_tests.rs:326 | test | test-scope .expect("canonical temp dir")
R1 | src/watch/manager_event_tests.rs:327 | test | test-scope .expect("project dir")
R1 | src/watch/manager_event_tests.rs:328 | test | test-scope .expect("lib file")
R1 | src/watch/manager_event_tests.rs:329 | test | test-scope .expect("git dir")
R1 | src/watch/manager_event_tests.rs:330 | test | test-scope .expect("git config")
R1 | src/watch/state.rs:209 | test | test-scope .expect("unpoisoned")
R1 | src/watch/scope_tests.rs:10 | test | test-scope .expect("project dir")
R1 | src/watch/scope_tests.rs:12 | test | test-scope .expect("git dir")
R1 | src/watch/scope_tests.rs:13 | test | test-scope .expect("data dir")
R1 | src/watch/scope_tests.rs:14 | test | test-scope .expect("target dir")
R1 | src/watch/scope_tests.rs:15 | test | test-scope .expect("src dirs")
R1 | src/watch/scope_tests.rs:16 | test | test-scope .expect("gitignore")
R1 | src/watch/scope_tests.rs:17 | test | test-scope .expect("lib file")
R1 | src/watch/scope_tests.rs:43 | test | test-scope .expect("project dir")
R1 | src/watch/scope_tests.rs:53 | test | test-scope .expect("project dir")
R1 | src/watch/scope_tests.rs:55 | test | test-scope .expect("nested dirs")
R1 | src/watch/scope_tests.rs:56 | test | test-scope .expect("root gitignore")
R1 | src/watch/scope_tests.rs:57 | test | test-scope .expect("nested ignore")
R1 | src/watch/scope_tests.rs:58 | test | test-scope .expect("nested gitignore")
R1 | src/watch/scope_tests.rs:71 | test | test-scope .expect("project dir")
R1 | src/watch/scope_tests.rs:74 | test | test-scope .expect("dir")
R1 | src/watch/scope_tests.rs:81 | test | test-scope .expect("more entries than the cap must report a limit")
R1 | src/watch/scope_tests.rs:98 | test | test-scope .expect("project dir")
R1 | src/watch/scope_tests.rs:100 | test | test-scope .expect("nested dirs")
R1 | src/watch/scope_tests.rs:106 | test | test-scope .expect("a directory at the depth cap with children must report a limit")
R1 | src/watch/scope_tests.rs:123 | test | test-scope .expect("project dir")
R1 | src/watch/scope_tests.rs:125 | test | test-scope .expect("src dir")
R1 | src/watch/scope_tests.rs:126 | test | test-scope .expect("lib file")
R1 | src/watch/tests.rs:15 | test | test-scope .expect("create temp dir")
R1 | src/watch/tests.rs:17 | test | test-scope .expect("create activation dir")
R1 | src/watch/tests.rs:25 | test | test-scope .expect("create parent dirs")
R1 | src/watch/tests.rs:27 | test | test-scope .expect("write file")
R1 | src/watch/tests.rs:65 | test | test-scope .expect("delete file")
R1 | src/project/root.rs:46 | test | test-scope .expect("temp dir")
R1 | src/project/root.rs:47 | test | test-scope .expect("valid root")
R1 | src/project/root.rs:53 | test | test-scope .expect("tempdir canonicalizes")
R1 | src/project/root.rs:59 | test | test-scope .expect("temp dir")
R1 | src/project/root.rs:61 | test | test-scope .expect("write fixture file")
R1 | src/project/root.rs:70 | test | test-scope .expect("temp dir")
R1 | src/project/root.rs:80 | test | test-scope .expect("temp dir")
R1 | src/project/root.rs:84 | test | test-scope panic!(..)
R1 | src/project/activation_lock.rs:78 | test | test-scope .expect("temp dir")
R1 | src/project/activation_lock.rs:79 | test | test-scope .expect("resolve root")
R1 | src/project/activation_lock.rs:80 | test | test-scope .expect("enable")
R1 | src/project/activation_lock.rs:82 | test | test-scope .expect("open writer")
R1 | src/project/activation_lock.rs:85 | test | test-scope .expect("leave frames in the WAL")
R1 | src/project/activation_lock.rs:86 | test | test-scope .expect("open reader")
R1 | src/project/activation_lock.rs:87 | test | test-scope .expect("begin read")
R1 | src/project/activation_lock.rs:90 | test | test-scope .expect("pin a read snapshot on the WAL")
R1 | src/project/activation_tests.rs:6 | test | test-scope .expect("create activation dir")
R1 | src/project/activation_tests.rs:13 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:14 | test | test-scope .expect("valid root")
R1 | src/project/activation_tests.rs:16 | test | test-scope .expect("enable")
R1 | src/project/activation_tests.rs:19 | test | test-scope .expect("stat activation dir")
R1 | src/project/activation_tests.rs:33 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:35 | test | test-scope .expect("pre-create wider")
R1 | src/project/activation_tests.rs:36 | test | test-scope .expect("widen")
R1 | src/project/activation_tests.rs:38 | test | test-scope .expect("valid root")
R1 | src/project/activation_tests.rs:39 | test | test-scope .expect("enable")
R1 | src/project/activation_tests.rs:41 | test | test-scope .expect("stat")
R1 | src/project/activation_tests.rs:47 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:49 | test | test-scope .expect("active project")
R1 | src/project/activation_tests.rs:55 | test | test-scope .expect("tempdir canonicalizes")
R1 | src/project/activation_tests.rs:62 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:69 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:71 | test | test-scope .expect("active project")
R1 | src/project/activation_tests.rs:77 | test | test-scope .expect("tempdir canonicalizes")
R1 | src/project/activation_tests.rs:83 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:86 | test | test-scope .expect("create nested dir")
R1 | src/project/activation_tests.rs:88 | test | test-scope .expect("write fixture file")
R1 | src/project/activation_tests.rs:90 | test | test-scope .expect("active project")
R1 | src/project/activation_tests.rs:96 | test | test-scope .expect("tempdir canonicalizes")
R1 | src/project/activation_tests.rs:102 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:111 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:128 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:130 | test | test-scope .expect("create link target")
R1 | src/project/activation_tests.rs:131 | test | test-scope .expect("create planning dir")
R1 | src/project/activation_tests.rs:136 | test | test-scope .expect("create symlink")
R1 | src/project/activation_tests.rs:146 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:147 | test | test-scope .expect("resolve root")
R1 | src/project/activation_tests.rs:149 | test | test-scope .expect("enable succeeds")
R1 | src/project/activation_tests.rs:153 | test | test-scope .expect("now active")
R1 | src/project/activation_tests.rs:161 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:162 | test | test-scope .expect("resolve root")
R1 | src/project/activation_tests.rs:163 | test | test-scope .expect("first enable")
R1 | src/project/activation_tests.rs:172 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:173 | test | test-scope .expect("resolve root")
R1 | src/project/activation_tests.rs:174 | test | test-scope .expect("enable")
R1 | src/project/activation_tests.rs:176 | test | test-scope .expect("disable succeeds")
R1 | src/project/activation_tests.rs:189 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:190 | test | test-scope .expect("resolve root")
R1 | src/project/activation_tests.rs:191 | test | test-scope .expect("disable succeeds even when never enabled")
R1 | src/project/activation_tests.rs:198 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:200 | test | test-scope .expect("create link target")
R1 | src/project/activation_tests.rs:202 | test | test-scope .expect("create symlink")
R1 | src/project/activation_tests.rs:203 | test | test-scope .expect("resolve root")
R1 | src/project/activation_tests.rs:227 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:231 | test | test-scope .expect("create nested dir")
R1 | src/project/activation_tests.rs:232 | test | test-scope .expect("canonicalize root")
R1 | src/project/activation_tests.rs:239 | test | test-scope .expect("toggler removes the marker")
R1 | src/project/activation_tests.rs:240 | test | test-scope .expect("toggler recreates the marker")
R1 | src/project/activation_tests.rs:249 | test | test-scope panic!(..)
R1 | src/project/activation_tests.rs:255 | test | test-scope .expect("toggler thread joins")
R1 | src/project/activation_tests.rs:261 | test | test-scope .expect("valid root")
R1 | src/project/activation_tests.rs:263 | test | test-scope .expect("write .gitignore")
R1 | src/project/activation_tests.rs:269 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:270 | test | test-scope .expect("valid root")
R1 | src/project/activation_tests.rs:277 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:285 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:292 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:308 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:320 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:329 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:338 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:346 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:353 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:361 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:368 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:376 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:383 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:391 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:398 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:406 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:407 | test | test-scope .expect("valid root")
R1 | src/project/activation_tests.rs:409 | test | test-scope .expect("a directory where the ignore file belongs")
R1 | src/project/activation_tests.rs:421 | test | test-scope panic!(..)
R1 | src/project/activation_tests.rs:427 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:428 | test | test-scope .expect("cap fits usize")
R1 | src/project/activation_tests.rs:445 | test | test-scope panic!(..)
R1 | src/project/activation_tests.rs:448 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:456 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:457 | test | test-scope .expect("cap fits usize")
R1 | src/project/activation_tests.rs:472 | test | test-scope .expect("read .gitignore")
R1 | src/project/activation_tests.rs:491 | test | test-scope .expect("temp dir")
R1 | src/project/activation_tests.rs:493 | test | test-scope .expect("a missing path is absent")
R1 | src/project/config.rs:134 | test | test-scope .expect("tempdir")
R1 | src/project/config.rs:136 | test | test-scope .expect("create dir")
R1 | src/project/config.rs:138 | test | test-scope .expect("write")
R1 | src/project/config.rs:140 | test | test-scope .expect("load should succeed")
R1 | src/project/config.rs:155 | test | test-scope .expect("tempdir")
R1 | src/project/config.rs:158 | test | test-scope .expect("load missing")
R1 | src/project/config.rs:166 | test | test-scope .expect("create dir")
R1 | src/project/config.rs:168 | test | test-scope .expect("write")
R1 | src/project/config.rs:173 | test | test-scope panic!(..)
R1 | src/project/config.rs:189 | test | test-scope .expect("tempdir")
R1 | src/project/config.rs:191 | test | test-scope .expect("create dir")
R1 | src/project/config.rs:196 | test | test-scope .expect("write")
R1 | src/project/config.rs:207 | test | test-scope .expect("tempdir")
R1 | src/project/config.rs:209 | test | test-scope .expect("create dir")
R1 | src/project/config.rs:210 | test | test-scope .expect("write")
R1 | src/project/database_path.rs:24 | test | test-scope .expect("temp dir")
R1 | src/project/database_path.rs:26 | test | test-scope .expect("create activation dir")
R1 | src/project/database_path.rs:27 | test | test-scope .expect("valid root")
R1 | src/project/database_path.rs:35 | test | test-scope .expect("tempdir canonicalizes")
R1 | src/watch/state.rs:213 | test | test-scope std::panic::panic_any(..) deliberately panics while holding the mutex to exercise poison recovery
R2 | src/store/migrations_tests.rs:179 | test | `.ok()` discards a rusqlite::Result; a query failure becomes None and the underlying error is lost instead of reaching the assertion
R5 | src/store/netfs.rs:70 | prod | StoreError::NetworkFilesystem is never asserted by a test: netfs_tests.rs:15 only asserts the negative (a local path is not that variant) and no test drives reject_network_filesystem to its positive branch
R5 | src/store/connection.rs:295 | prod | open_read_write's Open arm (Connection::open_with_flags failure) has no test asserting StoreError::Open; the only Open assertion (connection_tests.rs:41) covers open_read_only
R5 | src/store/connection.rs:324 | prod | open_read_only's Configure arms after the open (busy_timeout at 324, function registration at 326) have no test; the Configure assertion at connection_tests.rs:127 goes through open_read_write
R5 | src/store/connection.rs:84 | prod | discard_corrupt_database's only failure path (remove_file error other than NotFound) has no test at all
R5 | src/project/activation_lock.rs:21 | prod | ActivationError::Database (open_read_write failure while acquiring the disable lock) is constructed only here and never asserted by a test
R6 | src/store/connection.rs:48 | prod | NetworkFilesystemCheck tells the operator to check the "filesystem inspection command (stat/fsutil/mountinfo)", but the code runs neither stat nor fsutil: Linux reads /proc/self/mountinfo (netfs.rs:86) and macOS runs /sbin/mount (netfs.rs:203); the remedy names commands that do not exist on either supported platform
R6 | src/project/config.rs:94 | prod | load_or_default logs every ConfigError as "ignoring malformed project config", including ConfigError::Io (unreadable file, a directory at the path, or a file over the 1 MiB cap); a well-formed config can be reported as malformed and the operator is sent to the wrong fix
R6 | src/project/activation.rs:42 | prod | DatabaseLock's message says "could not take an exclusive lock on the database while disabling", but lock_error maps every non-BUSY/LOCKED SQLite failure (for example SQLITE_IOERR from PRAGMA wal_checkpoint) to this variant at activation_lock.rs:49, so an I/O failure is reported to the caller as a lock/contention failure
R7 | src/store/sql_functions.rs:53 | prod | line_range (and line_count at line 97) accepts any TEXT value agent SQL can synthesize with no function-level cap; SQLITE_MAX_VALUE_BYTES (1 GB) is the only bound, and slice_lines(..).to_owned() materializes the requested slice before the query tool's response budget applies, so line_count(hex(randomblob(400000000))) scans an ~800 MB value and line_range can copy it into a new String
R8 | src/store/connection.rs:19 | prod | public StoreError::Open variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:21 | prod | public StoreError::Configure variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:23 | prod | public StoreError::Migration variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:25 | prod | public StoreError::Symlink variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:27 | prod | public StoreError::Inspect variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:32 | prod | public StoreError::WalUnavailable variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:34 | prod | public StoreError::Permissions variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:42 | prod | public StoreError::NetworkFilesystem variant has no doc comment (authoritative missing_docs list)
R8 | src/store/connection.rs:51 | prod | public StoreError::NetworkFilesystemCheck variant has no doc comment (authoritative missing_docs list)
R8 | src/store/migrations.rs:17 | prod | public MigrationError::ReadVersion variant has no doc comment (authoritative missing_docs list)
R8 | src/store/migrations.rs:19 | prod | public MigrationError::Apply variant has no doc comment (authoritative missing_docs list)
R8 | src/store/migrations.rs:25 | prod | public MigrationError::UnsupportedVersion variant has no doc comment; the plain `//` comment above it is not a doc comment (authoritative missing_docs list)
R8 | src/project/root.rs:8 | prod | public RootError::Unresolvable variant has no doc comment (authoritative missing_docs list)
R8 | src/project/root.rs:10 | prod | public RootError::NotADirectory variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:22 | prod | public ActivationError::Root variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:24 | prod | public ActivationError::NotActive variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:26 | prod | public ActivationError::SymlinkedActivationPath variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:28 | prod | public ActivationError::Io variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:30 | prod | public ActivationError::Database variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:35 | prod | public ActivationError::DatabaseBusy variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:41 | prod | public ActivationError::CheckpointIncomplete variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:43 | prod | public ActivationError::DatabaseLock variant has no doc comment (authoritative missing_docs list)
R8 | src/project/activation.rs:237 | prod | public fields `path` and `reason` of GitignoreOutcome::NotWritten have no doc comments (authoritative missing_docs list)
R8 | src/watch/scope.rs:42 | prod | WatchScopeLimit is re-exported from watch but has no consumer outside scope.rs: it is constructed and read only there, and WatchScope.limit has no external reader
R8 | src/watch/manager.rs:85 | prod | WatchManager::new has no non-test caller (only watch/manager_event_tests.rs); production builds use Default
R8 | src/watch/state.rs:104 | prod | WatchState::unreconciled_count has no non-test caller; only its own cfg(test) test calls it, while production readers use ProjectWatchState::unreconciled_count or has_unreconciled_events
R8 | src/watch/types.rs:67 | prod | ProjectWatchState::new has zero call sites anywhere; the derived Default is what callers use
R10 | src/store/connection_tests.rs:210-221 | test | near-identical 0o644 fixture setup (comment + OpenOptions create_new 0o644 + set_permissions) is duplicated at src/store/connection_tests.rs:317-330; a shared wide-permissions fixture helper would keep both tests on one setup
R10 | src/store/connection_tests.rs:386-397 | test | capture-liveness block (capture_warns closure + liveness warn + two assertions) is duplicated at src/store/netfs_tests.rs:206-217 with only the sanity message and the checked rejection differing; it belongs in store/test_capture.rs as a helper
R12 | src/store/connection.rs:112 | prod | warn logs the raw absolute database path (`path = %path.display()`) when lstat fails; R12 treats paths as user data. This is a store guard refusal but it carries none of the deliberate-diagnostic comments decision (b) requires (contrast connection.rs:120-124 and :250-253), so the 2026-10-16 path-logging decision does not cover it; the rest of the chunk uses redacted_path for correlation logs
R12 | src/store/connection.rs:153 | prod | canonicalize-fallback warn logs the raw absolute path (`path = %path.display()`) during open; it is neither a documented one-shot rejection/cleanup site under decision (b) (no deliberate-raw-path comment) nor redacted under decision (a)
R12 | src/watch/manager.rs:117 | prod | `error = %error` on the notify event-loop failure; notify::Error Display appends `about [paths]` when paths are present (notify 8.2.0 error.rs:120-124), so the raw watched path is logged
R12 | src/watch/manager.rs:191 | prod | failed-to-watch warn pairs a redacted root_id with `error = %error`; notify::Error Display appends `about [paths]`, so the absolute project path the code tried to redact is logged anyway
R12 | src/watch/manager.rs:284 | prod | unwatch-failure warn pairs a redacted root_id with `error = %error`; notify::Error Display embeds the raw path, defeating the redaction
R12 | src/watch/scope.rs:245 | prod | unwatch-failure warn pairs a redacted dir_id with `error = %error`; notify::Error Display embeds the raw directory path
R12 | src/watch/scope.rs:253 | prod | watch-failure warn pairs a redacted dir_id with `error = %error`; notify::Error Display embeds the raw directory path
R12 | src/watch/scope.rs:115 | prod | walker-error warn logs `error = %error`; ignore::Error::WithPath Display is `{path}: {err}` (ignore 0.4.33 lib.rs:338-340), so the raw directory path is logged
R12 | src/watch/manager.rs:414 | prod | poisoned-mutex error is a bare message with no structured context (no root_id, mutex, or operation field), unlike the other failure logs in this manager

## Chunk 5

R2 | src/sync/sweep_tests.rs:461 | test | `first_walked_file` uses `filter_map(Result::ok)` on the walker, discarding every `ignore::Error`; a walk failure is indistinguishable from "no file", so the helper can report the fixture-file expect instead of the real walk error.
R5 | src/sync/manager.rs:38 | prod | `SyncError::Revision` has no test asserting it and is never constructed in `src/` (the `#[from]` conversion is dead); no failure path pins this variant.
R5 | src/sync/manager.rs:55 | prod | `SyncError::Read` has no test asserting it and is never constructed in `src/`; no failure path pins this variant.
R5 | src/sync/discovery.rs:43 | prod | `DiscoveryError::Walk` is never returned from `discover` (walker entry errors become skipped rows at line 169) and no test asserts it as a reason or an error.
R5 | src/sync/discovery.rs:51 | prod | `DiscoveryError::NotRelative` has no test asserting that exact variant/reason (the `strip_prefix` failure arm at line 179 is uncovered).
R5 | src/sync/discovery.rs:56 | prod | `DiscoveryError::NonUtf8Path` is exercised only by `a_non_utf8_filename_is_skipped_not_a_panic_or_a_whole_walk_failure`, which asserts `skipped.len() == 1` and never the reason/variant.
R5 | src/sync/manager_health.rs:227 | prod | public `SourceSyncManager::reconcile` has no failure-path test; `manager_reconcile_timeout_propagates_so_ensure_current_can_mark_desynced` (src/sync/timeout_tests.rs:175) drives `sync_with_barrier_with_deadline` directly and never calls `reconcile`.
R5 | src/sync/manager_health.rs:299 | prod | `expected_current` is read once before the barrier loop and reused by every iteration; if iteration 1 publishes a revision and an event arriving during that publish changes the same file again, iteration 2 sees a changed file, publishes against the stale baseline, and hits `RevisionError::StaleBaseline` (src/sync/revision.rs:252) — the call fails (and on the Healthy path `drain_events` marks the watcher Desynced) instead of reconciling in the same call. `barrier_loop_catches_events_during_reconciliation` (src/sync/manager_tests.rs:226) makes both writes before the call, so it never runs a second publishing iteration and does not cover this arm.
R5 | src/sync/manager_orchestration.rs:78 | prod | `discard_corrupt_database` failure on the corrupt-open recovery path returns `ErrorData` with no test asserting that error.
R5 | src/sync/manager_orchestration.rs:95 | prod | the non-corruption open failure is exercised only by `an_unopenable_database_surfaces_as_an_error_not_a_panic`, which asserts `is_err()` and never the message or `data`.
R5 | src/sync/manager_orchestration.rs:132 | prod | `discard_corrupt_database` failure on the stale-metadata recovery path returns `ErrorData` with no test asserting that error.
R5 | src/sync/manager_orchestration.rs:141 | prod | `ProjectMetaError::Other` from `ensure_project_row` is returned unchanged; no test asserts this arm (`RootMismatch`/`IncompatibleVersion` are covered).
R5 | src/sync/manager_meta.rs:43 | prod | the "purging prior-session findings" `ErrorData` has no test asserting it (the purge-failure arm is uncovered).
R5 | src/sync/manager_meta.rs:189 | prod | the "recording project metadata" `ErrorData` has no test asserting it.
R5 | src/sync/manager_meta.rs:202 | prod | the "reading project metadata" `ErrorData` has no test asserting it.
R5 | src/sync/manager_meta.rs:274 | prod | `open_read_write` failure while recreating the database after corruption/staleness has no test asserting the "recreating the project database" error.
R5 | src/sync/manager_health.rs:101 | prod | the post-verification `reconcile` failure arm returns `ErrorData::internal_error` with no test asserting it (the `verify_and_drain` drain-failure path is uncovered).
R5 | src/sync/manager_health.rs:154 | prod | the `sweep_tree` failure arm of `healthy_path` is never tested; no test asserts the `ErrorData` (with `sync_budget_exhausted` payload) produced there.
R5 | src/sync/manager_health.rs:202 | prod | the `drain_events` error is asserted only by `a_failed_incremental_reconcile_marks_the_watcher_desynced`, which checks `is_err()` plus `Desynced` and never the message or `data`.
R5 | src/sync/manager_health.rs:267 | prod | `revision_after_sync`'s "reading the current revision" error arm has no test asserting it.
R5 | src/sync/manager_health.rs:274 | prod | `revision_after_sync`'s "no current revision found after sync" error arm has no test asserting it.
R5 | src/sync/sweep_reconcile.rs:138 | prod | the `apply_sweep_report` failure arm (sweep walk succeeded, reconcile failed) is never tested; only the drain failure through `manager_health.rs:202` is covered.
R6 | src/sync/hash.rs:20 | prod | `HashError::TooLarge`'s Display claims `{path} is {observed} bytes`, but `observed` is the bounded read length (capped at `max_bytes + 1`); a 10 GB file with a 1 MB cap is reported as "1048577 bytes", which is not the file's size.
R6 | src/sync/sweep_reconcile.rs:59 | prod | `sweep_failed` logs "stat sweep failed" for failures of `apply_sweep_report` too (sweep_reconcile.rs:138), where the walk succeeded and the reconcile/publish failed — the log misnames what failed.
R8 | src/sync/manager.rs:28 | prod | pub field `SyncedProject::database_path` has no doc comment (rustc missing_docs).
R8 | src/sync/manager.rs:29 | prod | pub field `SyncedProject::revision_id` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:15 | prod | pub variant `FileKind::Indexed` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:16 | prod | pub variant `FileKind::Binary` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:23 | prod | pub field `DiscoveredFile::relative_path` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:24 | prod | pub field `DiscoveredFile::absolute_path` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:25 | prod | pub field `DiscoveredFile::kind` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:34 | prod | pub field `SkippedFile::absolute_path` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:35 | prod | pub field `SkippedFile::reason` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:43 | prod | pub variant `DiscoveryError::Walk` has no doc comment and no reference outside `sync::discovery` (it is only stringified as a skipped reason).
R8 | src/sync/discovery.rs:45 | prod | pub variant `DiscoveryError::Read` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:46 | prod | pub field `DiscoveryError::Read::path` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:48 | prod | pub field `DiscoveryError::Read::source` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:51 | prod | pub variant `DiscoveryError::NotRelative` has no doc comment and no reference outside `sync::discovery`.
R8 | src/sync/discovery.rs:56 | prod | pub variant `DiscoveryError::NonUtf8Path` is documented but has no reference outside `sync::discovery` (only a skipped-reason string).
R8 | src/sync/discovery.rs:58 | prod | pub variant `DiscoveryError::TimeBudgetExceeded` has no doc comment (rustc missing_docs).
R8 | src/sync/discovery.rs:58 | prod | pub field `DiscoveryError::TimeBudgetExceeded::elapsed_ms` has no doc comment (rustc missing_docs reports the field separately).
R8 | src/sync/hash.rs:12 | prod | pub variant `HashError::Read` has no doc comment (rustc missing_docs).
R8 | src/sync/hash.rs:13 | prod | pub field `HashError::Read::path` has no doc comment (rustc missing_docs).
R8 | src/sync/hash.rs:15 | prod | pub field `HashError::Read::source` has no doc comment (rustc missing_docs).
R8 | src/sync/hash.rs:22 | prod | pub field `HashError::TooLarge::path` has no doc comment (rustc missing_docs).
R8 | src/sync/hash.rs:23 | prod | pub field `HashError::TooLarge::observed` has no doc comment (rustc missing_docs).
R8 | src/sync/hash.rs:24 | prod | pub field `HashError::TooLarge::limit` has no doc comment (rustc missing_docs).
R10 | src/sync/manager_meta.rs:36-46 | prod | `purge_prior_session_findings`'s body is copy-pasted into the `cfg(test)` `purge_prior_session_findings_with` (53-66); one helper taking the session id would serve both.
R10 | src/sync/manager_tests.rs:243-252 | test | the `with_verified_read` content-read + assert block is duplicated at 276-285 (and 306-317, 390-403); a `read_indexed_content(&SyncedProject)` helper would collapse them.
R10 | src/sync/manager_observability_tests.rs:193-203 | test | the rebuilt-index content query + assert block is duplicated at 309-319 (and 257-267); a shared assertion helper would collapse them.
R12 | src/sync/manager_meta.rs:233 | prod | every failed manager-path publish is warn-logged twice: `publish_cas.rs:100` calls `publish_log::log_outcome`, which logs "publish failed" (src/sync/publish_log.rs:16), then `map_publish_error` logs the same failure again here.
R12 | src/sync/analyze.rs:158 | prod | per-file parse/status logs pass the raw project-relative path (`path = relative_path`) at lines 67, 72, 79, 118, 135, 158, 181; only 118/158 are warn-level failures, but all are routine sync logs outside the 2026-10-16 one-shot carve-out, and every other sync module logs paths through `util::redacted_path`.
R1 | src/sync/manager_tests.rs:33 | test | test uses `.expect("read content");`.
R1 | src/sync/manager_tests.rs:35 | test | test uses `assert!(content_a.expect("read file back").contains("42"));`.
R1 | src/sync/manager_tests.rs:51 | test | test uses `.expect("read content");`.
R1 | src/sync/manager_tests.rs:53 | test | test uses `assert!(content_b.expect("read file back").contains("99"));`.
R1 | src/sync/manager_tests.rs:106 | test | test uses `.expect("read reconciled file state");`.
R1 | src/sync/manager_tests.rs:122 | test | test uses `.expect("read count");`.
R1 | src/sync/manager_tests.rs:133 | test | test uses `.expect("read count");`.
R1 | src/sync/manager_tests.rs:149 | test | test uses `.expect("read count");`.
R1 | src/sync/manager_tests.rs:152 | test | test uses `fs::remove_file(project.path().join("to_delete.rs")).expect("delete file");`.
R1 | src/sync/manager_tests.rs:160 | test | test uses `.expect("read count");`.
R1 | src/sync/manager_tests.rs:251 | test | test uses `.expect("read content");`.
R1 | src/sync/manager_tests.rs:252 | test | test uses `assert!(content.expect("read file back").contains("pub fn c()"));`.
R1 | src/sync/manager_tests.rs:284 | test | test uses `.expect("read content");`.
R1 | src/sync/manager_tests.rs:285 | test | test uses `assert!(content.expect("read file back").contains("pub fn b()"));`.
R1 | src/sync/manager_tests.rs:299 | test | test uses `fs::remove_file(project.path().join("lib.rs")).expect("delete file");`.
R1 | src/sync/manager_tests.rs:314 | test | test uses `.expect("read content");`.
R1 | src/sync/manager_tests.rs:317 | test | test uses `.expect("read file back")`.
R1 | src/sync/manager_tests.rs:387 | test | test uses `.expect("read count");`.
R1 | src/sync/manager_tests.rs:398 | test | test uses `.expect("read content");`.
R1 | src/sync/manager_tests.rs:401 | test | test uses `.expect("read file back")`.
R1 | src/sync/manager_tests.rs:418 | test | test uses `let connection = rusqlite::Connection::open(&synced.database_path).expect("raw open");`.
R1 | src/sync/manager_tests.rs:421 | test | test uses `.expect("rewind user_version to 1");`.
R1 | src/sync/manager_tests.rs:432 | test | test uses `.expect("read version");`.
R1 | src/sync/manager_tests.rs:442 | test | test uses `.expect("read count");`.
R1 | src/sync/hash.rs:196 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/hash.rs:198 | test | test uses `std::fs::write(&path, b"fn a() {}").expect("write fixture file");`.
R1 | src/sync/hash.rs:200 | test | test uses `let identity = hash_file("a.rs", &path, 1024).expect("hash file");`.
R1 | src/sync/hash.rs:209 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/hash.rs:212 | test | test uses `std::fs::write(&path, bytes).expect("write fixture file");`.
R1 | src/sync/hash.rs:215 | test | test uses `.expect("a file under the cap hashes");`.
R1 | src/sync/hash.rs:222 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/hash.rs:225 | test | test uses `std::fs::write(&path, bytes).expect("write fixture file");`.
R1 | src/sync/hash.rs:228 | test | test uses `hash_file("at-cap.rs", &path, bytes.len() as u64).expect("the cap is inclusive");`.
R1 | src/sync/hash.rs:235 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/hash.rs:237 | test | test uses `std::fs::write(&path, b"ninebytes").expect("write fixture file");`.
R1 | src/sync/hash.rs:239 | test | test uses `let error = hash_file("over.rs", &path, 8).expect_err("over-cap file is refused");`.
R1 | src/sync/hash.rs:254 | test | test uses `other => panic!("expected TooLarge, got {other:?}"),`.
R1 | src/sync/hash.rs:260 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/hash.rs:263 | test | test uses `let error = hash_file("missing.rs", &path, 1024).expect_err("a missing file is unreadable");`.
R1 | src/sync/manager_meta_tests.rs:19 | test | test uses `let dir = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/manager_meta_tests.rs:21 | test | test uses `let connection = store::open_read_write(&db_path).expect("open db");`.
R1 | src/sync/manager_meta_tests.rs:28 | test | test uses `let revision = current_revision_id(&connection).expect("query");`.
R1 | src/sync/manager_meta_tests.rs:36 | test | test uses `ensure_project_row(&mut connection, root).expect("first ensure");`.
R1 | src/sync/manager_meta_tests.rs:38 | test | test uses `ensure_project_row(&mut connection, root).expect("idempotent on second call");`.
R1 | src/sync/manager_meta_tests.rs:45 | test | test uses `ensure_project_row(&mut connection, first_root).expect("first ensure");`.
R1 | src/sync/manager_meta_tests.rs:47 | test | test uses `let err = ensure_project_row(&mut connection, second_root).expect_err("must error");`.
R1 | src/sync/manager_meta_tests.rs:74 | test | test uses `ensure_project_row(&mut connection, root).expect("first ensure");`.
R1 | src/sync/manager_meta_tests.rs:80 | test | test uses `.expect("stamp an unsupported version");`.
R1 | src/sync/manager_meta_tests.rs:81 | test | test uses `let err = ensure_project_row(&mut connection, root).expect_err("must error");`.
R1 | src/sync/manager_meta_tests.rs:120 | test | test uses `.expect("seed finding");`.
R1 | src/sync/manager_meta_tests.rs:126 | test | test uses `Uuid::parse_str("33333333-3333-3333-3333-333333333333").expect("new uuid"),`.
R1 | src/sync/manager_meta_tests.rs:128 | test | test uses `.expect("purge");`.
R1 | src/sync/manager_meta_tests.rs:132 | test | test uses `.expect("select")`.
R1 | src/sync/manager_meta_tests.rs:134 | test | test uses `.expect("query")`.
R1 | src/sync/manager_meta_tests.rs:135 | test | test uses `.map(|value| value.expect("row"))`.
R1 | src/sync/manager_meta_tests.rs:157 | test | test uses `.expect("alive uuid")`.
R1 | src/sync/manager_meta_tests.rs:161 | test | test uses `.expect("insert alive");`.
R1 | src/sync/manager_meta_tests.rs:164 | test | test uses `Uuid::parse_str("44444444-4444-4444-4444-444444444444").expect("alive uuid"),`.
R1 | src/sync/manager_meta_tests.rs:166 | test | test uses `.expect("no-op purge");`.
R1 | src/sync/manager_meta_tests.rs:169 | test | test uses `.expect("count");`.
R1 | src/sync/manager_meta_tests.rs:185 | test | test uses `ensure_project_row(&mut connection, root).expect("first ensure");`.
R1 | src/sync/manager_meta_tests.rs:188 | test | test uses `Uuid::parse_str("55555555-5555-5555-5555-555555555555").expect("prior uuid");`.
R1 | src/sync/manager_meta_tests.rs:197 | test | test uses `.expect("seed stale finding");`.
R1 | src/sync/manager_meta_tests.rs:204 | test | test uses `Uuid::parse_str("55555555-5555-5555-5555-555555555555").expect("warm session"),`.
R1 | src/sync/manager_meta_tests.rs:206 | test | test uses `ensure_project_row(&mut connection, root).expect("warm-path with matching session");`.
R1 | src/sync/manager_meta_tests.rs:210 | test | test uses `.expect("count after warm");`.
R1 | src/sync/manager_meta_tests.rs:214 | test | test uses `Uuid::parse_str("66666666-6666-6666-6666-666666666666").expect("different session"),`.
R1 | src/sync/manager_meta_tests.rs:216 | test | test uses `ensure_project_row(&mut connection, root).expect("warm-path with new session");`.
R1 | src/sync/manager_meta_tests.rs:220 | test | test uses `.expect("count after cross-session");`.
R1 | src/sync/manager_meta_tests.rs:242 | test | test uses `let data = budget.data.expect("a spent budget carries a payload");`.
R1 | src/sync/manager_meta_tests.rs:244 | test | test uses `let remedy = data["remedy"].as_str().expect("remedy is a string");`.
R1 | src/sync/manager_meta_tests.rs:259 | test | test uses `wrapped.data.expect("wrapped budget")["code"],`.
R1 | src/sync/manager_meta_tests.rs:271 | test | test uses `raced.data.expect("the CAS race carries a payload")["code"],`.
R1 | src/sync/sweep_tests.rs:22 | test | test uses `.expect("clock is after the epoch")`.
R1 | src/sync/sweep_tests.rs:35 | test | test uses `let metadata = std::fs::metadata(root.join(path)).expect("stat fixture file");`.
R1 | src/sync/sweep_tests.rs:41 | test | test uses `i64::try_from(metadata.len()).expect("file length fits i64"),`.
R1 | src/sync/sweep_tests.rs:45 | test | test uses `.expect("update stored stat fingerprint");`.
R1 | src/sync/sweep_tests.rs:51 | test | test uses `sweep(connection, root, &deadline).expect("sweep succeeds")`.
R1 | src/sync/sweep_tests.rs:62 | test | test uses `.expect("read the current revision");`.
R1 | src/sync/sweep_tests.rs:72 | test | test uses `.expect("reconcile succeeds");`.
R1 | src/sync/sweep_tests.rs:126 | test | test uses `.expect("row exists");`.
R1 | src/sync/sweep_tests.rs:137 | test | test uses `std::fs::remove_file(project.path().join("b.rs")).expect("remove fixture");`.
R1 | src/sync/sweep_tests.rs:191 | test | test uses `.expect("bump stored mtime");`.
R1 | src/sync/sweep_tests.rs:194 | test | test uses `.expect("count revisions");`.
R1 | src/sync/sweep_tests.rs:210 | test | test uses `.expect("count revisions");`.
R1 | src/sync/sweep_tests.rs:243 | test | test uses `.expect("project registered by the first sync");`.
R1 | src/sync/sweep_tests.rs:255 | test | test uses `let connection = open_read_only(&second.database_path).expect("open served database");`.
R1 | src/sync/sweep_tests.rs:262 | test | test uses `.expect("row exists");`.
R1 | src/sync/sweep_tests.rs:288 | test | test uses `.expect("project registered by the first sync");`.
R1 | src/sync/sweep_tests.rs:321 | test | test uses `.expect("project registered by the first sync");`.
R1 | src/sync/sweep_tests.rs:334 | test | test uses `let connection = open_read_only(&second.database_path).expect("open served database");`.
R1 | src/sync/sweep_tests.rs:342 | test | test uses `.expect("pkg/main.py's edge");`.
R1 | src/sync/sweep_tests.rs:375 | test | test uses `.expect("project registered by the first sync");`.
R1 | src/sync/sweep_tests.rs:384 | test | test uses `let mut connection = open_read_write(&first.database_path).expect("open read-write");`.
R1 | src/sync/sweep_tests.rs:396 | test | test uses `let report = sweep(&connection, project.path(), &options.deadline).expect("sweep");`.
R1 | src/sync/sweep_tests.rs:404 | test | test uses `.expect("apply the sweep report");`.
R1 | src/sync/sweep_tests.rs:421 | test | test uses `.expect("the oversize target must have an existence row");`.
R1 | src/sync/sweep_tests.rs:433 | test | test uses `.expect("pkg/main.py's edge");`.
R1 | src/sync/sweep_tests.rs:446 | test | test uses `.expect("store an unknown size");`.
R1 | src/sync/sweep_tests.rs:463 | test | test uses `.expect("the walker yields the fixture file")`.
R1 | src/sync/sweep_tests.rs:468 | test | test uses `let project = tempfile::tempdir().expect("project dir");`.
R1 | src/sync/sweep_tests.rs:473 | test | test uses `std::fs::remove_dir_all(project.path().join("d")).expect("remove the directory");`.
R1 | src/sync/sweep_tests.rs:486 | test | test uses `let project = tempfile::tempdir().expect("project dir");`.
R1 | src/sync/sweep_tests.rs:489 | test | test uses `std::fs::remove_file(project.path().join("a.rs")).expect("delete the file");`.
R1 | src/sync/discovery_tests.rs:7 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:13 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:26 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:34 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:41 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:46 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:56 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:62 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:75 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:80 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:90 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:96 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:122 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:125 | test | test uses `std::fs::write(root.join(&bad_name), b"content").expect("write non-UTF-8-named file");`.
R1 | src/sync/discovery_tests.rs:126 | test | test uses `std::fs::write(root.join("good.txt"), b"content").expect("write a normal file");`.
R1 | src/sync/discovery_tests.rs:128 | test | test uses `let (files, skipped) = discover(root).expect("a bad filename must not fail the whole walk");`.
R1 | src/sync/discovery_tests.rs:152 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:156 | test | test uses `let (first, _skipped) = discover(root).expect("first discover");`.
R1 | src/sync/discovery_tests.rs:167 | test | test uses `let outside = tempfile::tempdir().expect("outside dir");`.
R1 | src/sync/discovery_tests.rs:169 | test | test uses `fs::write(&secret, [0_u8, 1, 2]).expect("write outside file");`.
R1 | src/sync/discovery_tests.rs:171 | test | test uses `std::fs::remove_file(root.join("lib.rs")).expect("remove the real file");`.
R1 | src/sync/discovery_tests.rs:172 | test | test uses `std::os::unix::fs::symlink(&secret, root.join("lib.rs")).expect("replace with a symlink");`.
R1 | src/sync/discovery_tests.rs:174 | test | test uses `let (second, _skipped) = discover(root).expect("second discover does not error or panic");`.
R1 | src/sync/discovery_tests.rs:196 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:206 | test | test uses `let (files, _skipped) = discover(root).expect("discover");`.
R1 | src/sync/discovery_tests.rs:220 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:222 | test | test uses `.expect_err("a missing file must be a reported error, not a panic");`.
R1 | src/sync/discovery_tests.rs:232 | test | test uses `let directory = tempfile::tempdir().expect("temp dir");`.
R1 | src/sync/discovery_tests.rs:234 | test | test uses `fs::create_dir(&dir).expect("create dir");`.
R1 | src/sync/discovery_tests.rs:236 | test | test uses `let error = sniff_kind(&dir).expect_err("reading a directory must error");`.
R1 | src/sync/analyze_tests.rs:44 | test | test uses `other => panic!("expected SyntaxErrors, got {other:?}"),`.
R1 | src/sync/manager_observability_tests.rs:14 | test | test uses `crate::store::open_read_write(&database).expect("open db")`.
R1 | src/sync/manager_observability_tests.rs:47 | test | test uses `let root = project.path().canonicalize().expect("canonical root");`.
R1 | src/sync/manager_observability_tests.rs:48 | test | test uses `let state = manager.watch_state_for(&root).expect("registered state");`.
R1 | src/sync/manager_observability_tests.rs:55 | test | test uses `let other = tempfile::tempdir().expect("other dir");`.
R1 | src/sync/manager_observability_tests.rs:69 | test | test uses `let state = manager.active_watch_state().expect("registered state");`.
R1 | src/sync/manager_observability_tests.rs:73 | test | test uses `.expect("reconcile with no events");`.
R1 | src/sync/manager_observability_tests.rs:81 | test | test uses `.expect("current revision");`.
R1 | src/sync/manager_observability_tests.rs:95 | test | test uses `let state = manager.active_watch_state().expect("registered state");`.
R1 | src/sync/manager_observability_tests.rs:100 | test | test uses `.expect("reconcile unchanged path");`.
R1 | src/sync/manager_observability_tests.rs:106 | test | test uses `.expect("revision count");`.
R1 | src/sync/manager_observability_tests.rs:123 | test | test uses `fs::write(database, b"corrupted mid-publish").expect("corrupt the database mid-publish");`.
R1 | src/sync/manager_observability_tests.rs:184 | test | test uses `fs::write(&database, b"this is not a sqlite database").expect("corrupt db");`.
R1 | src/sync/manager_observability_tests.rs:199 | test | test uses `.expect("read content from the rebuilt database");`.
R1 | src/sync/manager_observability_tests.rs:201 | test | test uses `content.expect("read file back").contains("pub fn a()"),`.
R1 | src/sync/manager_observability_tests.rs:229 | test | test uses `.expect("canonicalize project tempdir")`.
R1 | src/sync/manager_observability_tests.rs:233 | test | test uses `let conn = crate::store::open_read_write(&database).expect("open db");`.
R1 | src/sync/manager_observability_tests.rs:238 | test | test uses `.expect("rewrite stored root");`.
R1 | src/sync/manager_observability_tests.rs:252 | test | test uses `.expect("read stored root");`.
R1 | src/sync/manager_observability_tests.rs:263 | test | test uses `.expect("read content from the rebuilt database");`.
R1 | src/sync/manager_observability_tests.rs:265 | test | test uses `content.expect("read file back").contains("pub fn a()"),`.
R1 | src/sync/manager_observability_tests.rs:286 | test | test uses `let conn = crate::store::open_read_write(&database).expect("open db");`.
R1 | src/sync/manager_observability_tests.rs:291 | test | test uses `.expect("rewrite stored version");`.
R1 | src/sync/manager_observability_tests.rs:303 | test | test uses `.expect("read rebuilt version");`.
R1 | src/sync/manager_observability_tests.rs:315 | test | test uses `.expect("read content from the rebuilt database");`.
R1 | src/sync/manager_observability_tests.rs:317 | test | test uses `content.expect("read file back").contains("pub fn a()"),`.
R1 | src/sync/manager_observability_tests.rs:334 | test | test uses `let conn = crate::store::open_read_write(&database).expect("open db");`.
R1 | src/sync/manager_observability_tests.rs:340 | test | test uses `.expect("stamp the previous schema version");`.
R1 | src/sync/manager_observability_tests.rs:348 | test | test uses `.expect("read user_version");`.
R1 | src/sync/manager_observability_tests.rs:362 | test | test uses `fs::create_dir_all(&database).expect("make the db path a directory");`.
R1 | src/sync/manager_observability_tests.rs:387 | test | test uses `.expect("open database")`.
R1 | src/sync/manager_observability_tests.rs:394 | test | test uses `.expect("install failing triggers");`.
R1 | src/sync/manager_observability_tests.rs:397 | test | test uses `let state = manager.active_watch_state().expect("registered state");`.

## Chunk 6

R1 | src/sync/publish_acceptance_tests.rs:36 | test | `panic!` inside an `unwrap_or_else` fallback
R1 | src/sync/publish_acceptance_tests.rs:46 | test | `expect("prepare")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:49 | test | `expect("query")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:51 | test | `expect("collect")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:65 | test | `expect("resolved-edge query runs")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:71 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:72 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:73 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:111 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:137 | test | `expect("count logo evidence")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:158 | test | `expect("delete file")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:159 | test | `expect("remove old name")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:168 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:201 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:202 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:203 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:222 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_acceptance_tests.rs:231 | test | `panic!` inside an `unwrap_or_else` fallback
R1 | src/sync/publish_attempt.rs:288 | test | `expect("tempdir")` panics on failure
R1 | src/sync/publish_attempt.rs:308 | test | BTreeMap index `records["vanished.rs"]` with no in-function presence guard
R1 | src/sync/publish_attempt.rs:309 | test | BTreeMap index `records["vanished.rs"]` with no in-function presence guard
R1 | src/sync/publish_attempt.rs:314 | test | `expect("tempdir")` panics on failure
R1 | src/sync/publish_attempt.rs:316 | test | `expect("write file")` panics on failure
R1 | src/sync/publish_attempt.rs:336 | test | BTreeMap index `records["present.rs"]` with no in-function presence guard
R1 | src/sync/publish_edges_tests.rs:25 | test | `expect("prepare")` panics on failure
R1 | src/sync/publish_edges_tests.rs:34 | test | `expect("query")` panics on failure
R1 | src/sync/publish_edges_tests.rs:36 | test | `expect("collect")` panics on failure
R1 | src/sync/publish_edges_tests.rs:41 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:45 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:46 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:54 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:61 | test | explicit `panic!`
R1 | src/sync/publish_edges_tests.rs:74 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:81 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:82 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:90 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:96 | test | `expect("a resolved edge")` panics on failure
R1 | src/sync/publish_edges_tests.rs:102 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:108 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:109 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:117 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:123 | test | `expect("an edge for main.rs")` panics on failure
R1 | src/sync/publish_edges_tests.rs:130 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:135 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:136 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:143 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:152 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:172 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:190 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:191 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:199 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:220 | test | `expect("an edge for App.swift")` panics on failure
R1 | src/sync/publish_edges_tests.rs:227 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:231 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:232 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:239 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:246 | test | `expect("delete target")` panics on failure
R1 | src/sync/publish_edges_tests.rs:253 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:272 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:278 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_edges_tests.rs:279 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_edges_tests.rs:293 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_edges_tests.rs:308 | test | `panic!` inside an `unwrap_or_else` fallback
R1 | src/sync/publish_mutation_tests.rs:36 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:38 | test | `expect("discover")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:51 | test | `expect("a total exactly at the limit must be accepted, not rejected")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:86 | test | `expect("open racer db")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:93 | test | `expect("racer publish")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:100 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:102 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:105 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:112 | test | `expect("bootstrap publish")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:125 | test | `expect("open db (A)")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:132 | test | `expect("the test thread holds the receiver until the publish result arrives")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:139 | test | `expect(...)` panics on failure
R1 | src/sync/publish_mutation_tests.rs:175 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:179 | test | `expect("write non-UTF-8-named file")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:181 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:182 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:189 | test | `expect("publish must succeed despite one bad file")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:209 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:212 | test | `expect("write oversized file")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:214 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:215 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:227 | test | `expect("publish must succeed despite one oversized file")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:254 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:257 | test | `expect("write oversized file")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:259 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:260 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:273 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:292 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:309 | test | `expect("skipped row survives")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:322 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:326 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:327 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:335 | test | `expect("first publish indexes grow.rs")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:343 | test | `expect("count evidence before")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:347 | test | `expect("grow the file past the ceiling")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:358 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:382 | test | `expect("grow.rs row")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:395 | test | `expect("count evidence after")` panics on failure
R1 | src/sync/publish_mutation_tests.rs:406 | test | `expect("lib.rs row")` panics on failure
R1 | src/sync/publish_race_tests.rs:13 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:15 | test | `expect("discover")` panics on failure
R1 | src/sync/publish_race_tests.rs:22 | test | `expect("sample")` panics on failure
R1 | src/sync/publish_race_tests.rs:28 | test | `expect("build upserts")` panics on failure
R1 | src/sync/publish_race_tests.rs:44 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:46 | test | `expect("discover")` panics on failure
R1 | src/sync/publish_race_tests.rs:51 | test | `expect("sample")` panics on failure
R1 | src/sync/publish_race_tests.rs:53 | test | `expect("build upserts")` panics on failure
R1 | src/sync/publish_race_tests.rs:56 | test | `expect("remove the file")` panics on failure
R1 | src/sync/publish_race_tests.rs:57 | test | `expect("replace it with a directory")` panics on failure
R1 | src/sync/publish_race_tests.rs:68 | test | `expect_err("revalidation fails")` panics if the result is Ok
R1 | src/sync/publish_race_tests.rs:73 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:75 | test | `expect("discover")` panics on failure
R1 | src/sync/publish_race_tests.rs:80 | test | `expect("sample")` panics on failure
R1 | src/sync/publish_race_tests.rs:82 | test | `expect("build upserts")` panics on failure
R1 | src/sync/publish_race_tests.rs:83 | test | `expect("remove the file")` panics on failure
R1 | src/sync/publish_race_tests.rs:95 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:97 | test | `expect("discover")` panics on failure
R1 | src/sync/publish_race_tests.rs:102 | test | `expect("sample")` panics on failure
R1 | src/sync/publish_race_tests.rs:104 | test | `expect("build upserts")` panics on failure
R1 | src/sync/publish_race_tests.rs:111 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:113 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:114 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_race_tests.rs:130 | test | `expect("publish converges")` panics on failure
R1 | src/sync/publish_race_tests.rs:138 | test | `expect("read content")` panics on failure
R1 | src/sync/publish_race_tests.rs:150 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:152 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:155 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_race_tests.rs:162 | test | `expect("bootstrap publish")` panics on failure
R1 | src/sync/publish_race_tests.rs:168 | test | `expect("signal pause")` panics on failure
R1 | src/sync/publish_race_tests.rs:169 | test | `expect("wait for resume")` panics on failure
R1 | src/sync/publish_race_tests.rs:176 | test | `expect("open db (A)")` panics on failure
R1 | src/sync/publish_race_tests.rs:188 | test | `expect("A reached the barrier")` panics on failure
R1 | src/sync/publish_race_tests.rs:190 | test | `expect("open db (B)")` panics on failure
R1 | src/sync/publish_race_tests.rs:197 | test | `expect("B publishes first")` panics on failure
R1 | src/sync/publish_race_tests.rs:203 | test | `expect("signal resume")` panics on failure
R1 | src/sync/publish_race_tests.rs:206 | test | `expect("thread A joins")` panics on failure
R1 | src/sync/publish_race_tests.rs:207 | test | `expect("A converges after retrying")` panics on failure
R1 | src/sync/publish_race_tests.rs:213 | test | `expect("open db (final)")` panics on failure
R1 | src/sync/publish_race_tests.rs:225 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:227 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_race_tests.rs:230 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_race_tests.rs:237 | test | `expect("bootstrap publish")` panics on failure
R1 | src/sync/publish_race_tests.rs:243 | test | `expect("signal pause")` panics on failure
R1 | src/sync/publish_race_tests.rs:244 | test | `expect("wait for resume")` panics on failure
R1 | src/sync/publish_race_tests.rs:252 | test | `expect("open db (A)")` panics on failure
R1 | src/sync/publish_race_tests.rs:261 | test | `expect("A reached the barrier")` panics on failure
R1 | src/sync/publish_race_tests.rs:263 | test | `expect("open db (B)")` panics on failure
R1 | src/sync/publish_race_tests.rs:270 | test | `expect("B publishes")` panics on failure
R1 | src/sync/publish_race_tests.rs:273 | test | `expect("signal resume")` panics on failure
R1 | src/sync/publish_race_tests.rs:276 | test | `expect("thread A joins")` panics on failure
R1 | src/sync/publish_race_tests.rs:277 | test | `expect("A converges without error")` panics on failure
R1 | src/sync/publish_tests.rs:11 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:18 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:19 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:27 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_tests.rs:38 | test | `expect("prepare")` panics on failure
R1 | src/sync/publish_tests.rs:41 | test | `expect("query")` panics on failure
R1 | src/sync/publish_tests.rs:43 | test | `expect("collect")` panics on failure
R1 | src/sync/publish_tests.rs:55 | test | `expect("read content")` panics on failure
R1 | src/sync/publish_tests.rs:64 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:67 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:68 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:76 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_tests.rs:85 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:87 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:88 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:96 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_tests.rs:103 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_tests.rs:113 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:115 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:116 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:124 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_tests.rs:132 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_tests.rs:145 | test | `expect("read stored parser version")` panics on failure
R1 | src/sync/publish_tests.rs:151 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:154 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:155 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:162 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_tests.rs:165 | test | `expect("remove file")` panics on failure
R1 | src/sync/publish_tests.rs:173 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_tests.rs:184 | test | `expect("read content")` panics on failure
R1 | src/sync/publish_tests.rs:190 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:196 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:197 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:205 | test | `expect("publish")` panics on failure
R1 | src/sync/publish_tests.rs:215 | test | `expect("read file status")` panics on failure
R1 | src/sync/publish_tests.rs:228 | test | `expect("count structure evidence")` panics on failure
R1 | src/sync/publish_tests.rs:237 | test | `expect("project dir")` panics on failure
R1 | src/sync/publish_tests.rs:239 | test | `expect("db dir")` panics on failure
R1 | src/sync/publish_tests.rs:240 | test | `expect("open db")` panics on failure
R1 | src/sync/publish_tests.rs:247 | test | `expect("first publish")` panics on failure
R1 | src/sync/publish_tests.rs:255 | test | `expect("read file id")` panics on failure
R1 | src/sync/publish_tests.rs:262 | test | `expect("insert evidence")` panics on failure
R1 | src/sync/publish_tests.rs:264 | test | `expect("remove file")` panics on failure
R1 | src/sync/publish_tests.rs:271 | test | `expect("second publish")` panics on failure
R1 | src/sync/publish_tests.rs:275 | test | `expect("count evidence")` panics on failure
R1 | src/sync/sample_batch.rs:233 | prod | `self.discovered[idx]` indexed from a function parameter with no in-function bounds guard
R1 | src/sync/sample_batch.rs:266 | prod | `samples[idx]` indexed from a channel-supplied worker index with no in-function bounds guard
R1 | src/sync/sample_batch.rs:268 | prod | `self.discovered[idx]` indexed from a channel-supplied worker index with no in-function bounds guard
R1 | src/sync/sample_batch_tests.rs:8 | test | `expect("project dir")` panics on failure
R1 | src/sync/sample_batch_tests.rs:11 | test | `expect("discover")` panics on failure
R1 | src/sync/sample_tests.rs:32 | test | `expect("encode payload")` panics on failure
R1 | src/sync/sample_tests.rs:61 | test | `expect("payload serializes")` panics on failure
R1 | src/sync/sample_tests.rs:209 | test | `expect("tempdir")` panics on failure
R1 | src/sync/sample_tests.rs:212 | test | `expect("write fixture file")` panics on failure
R1 | src/sync/sample_tests.rs:223 | test | `expect("the per-file cap is inclusive")` panics on failure
R1 | src/sync/sample_tests.rs:232 | test | `expect("tempdir")` panics on failure
R1 | src/sync/sample_tests.rs:234 | test | `expect("write fixture file")` panics on failure
R1 | src/sync/sample_tests.rs:245 | test | `expect_err("an over-cap file is refused")` panics if the result is Ok
R1 | src/sync/sample_tests.rs:252 | test | explicit `panic!`
R1 | src/sync/sample_tests.rs:264 | test | `expect("tempdir")` panics on failure
R1 | src/sync/sample_tests.rs:270 | test | `expect("mkdir")` panics on failure
R1 | src/sync/sample_tests.rs:274 | test | `expect("write body")` panics on failure
R1 | src/sync/sample_tests.rs:301 | test | `expect("tight deadline should not fire here")` panics on failure
R1 | src/sync/sample_tests.rs:332 | test | `expect_err("total-byte cap must fire under this many large files")` panics if the result is Ok
R1 | src/sync/sample_tests.rs:340 | test | explicit `panic!`
R1 | src/sync/sample_tests.rs:349 | test | `expect("tempdir")` panics on failure
R1 | src/sync/sample_tests.rs:352 | test | `expect("write big file")` panics on failure
R1 | src/sync/sample_tests.rs:353 | test | `expect("write small file")` panics on failure
R1 | src/sync/sample_tests.rs:374 | test | `expect("an oversized file must be skipped, not fatal")` panics on failure
R1 | src/sync/sample_tests.rs:406 | test | `expect_err("deadline must fire before first sample completes")` panics if the result is Ok
R1 | src/sync/sample_tests.rs:409 | test | explicit `panic!`
R1 | src/sync/sample_tests.rs:433 | test | `expect("samples should succeed")` panics on failure
R1 | src/sync/sample_tests.rs:444 | test | `expect("tempdir")` panics on failure
R1 | src/sync/sample_tests.rs:456 | test | `expect("tempdir")` panics on failure
R1 | src/sync/sample_tests.rs:458 | test | `expect("write empty file")` panics on failure
R1 | src/sync/sample_tests.rs:473 | test | `expect("temp dir")` panics on failure
R1 | src/sync/sample_tests.rs:475 | test | `expect("write fixture")` panics on failure
R1 | src/sync/test_support.rs:21 | test | `expect("create parent dirs")` panics on failure
R1 | src/sync/test_support.rs:23 | test | `expect("write fixture file")` panics on failure
R1 | src/sync/test_support.rs:34 | test | `expect("create temp dir")` panics on failure
R1 | src/sync/test_support.rs:36 | test | `expect("create activation dir")` panics on failure
R1 | src/sync/test_support.rs:44 | test | `expect("project dir")` panics on failure
R1 | src/sync/test_support.rs:45 | test | `expect("db dir")` panics on failure
R1 | src/sync/test_support.rs:46 | test | `expect("open db")` panics on failure
R1 | src/sync/test_support.rs:52 | test | `expect("initial publish")` panics on failure
R1 | src/sync/test_support.rs:60 | test | `expect("sync succeeds")` panics on failure
R1 | src/sync/test_support.rs:67 | test | `expect("prepare")` panics on failure
R1 | src/sync/test_support.rs:70 | test | `expect("query")` panics on failure
R1 | src/sync/test_support.rs:72 | test | `expect("collect")` panics on failure
R1 | src/sync/timeout_tests.rs:30 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:34 | test | `expect_err("a spent budget must abort discovery")` panics if the result is Ok
R1 | src/sync/timeout_tests.rs:47 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:49 | test | `expect("discover")` panics on failure
R1 | src/sync/timeout_tests.rs:55 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:57 | test | `expect("discover")` panics on failure
R1 | src/sync/timeout_tests.rs:68 | test | explicit `panic!`
R1 | src/sync/timeout_tests.rs:78 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:80 | test | `expect("db dir")` panics on failure
R1 | src/sync/timeout_tests.rs:81 | test | `expect("open db")` panics on failure
R1 | src/sync/timeout_tests.rs:95 | test | `expect_err("a spent budget must abort the publish")` panics if the result is Ok
R1 | src/sync/timeout_tests.rs:116 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:117 | test | `expect("db dir")` panics on failure
R1 | src/sync/timeout_tests.rs:118 | test | `expect("open db")` panics on failure
R1 | src/sync/timeout_tests.rs:138 | test | `expect_err("a spent budget must abort reconcile")` panics if the result is Ok
R1 | src/sync/timeout_tests.rs:151 | test | `expect_err("a spent budget must abort the barrier loop")` panics if the result is Ok
R1 | src/sync/timeout_tests.rs:165 | test | `expect("no events means nothing to stall on")` panics on failure
R1 | src/sync/timeout_tests.rs:185 | test | `expect("manager has registered the watched project")` panics on failure
R1 | src/sync/timeout_tests.rs:197 | test | `expect_err("a spent budget must abort the barrier loop")` panics if the result is Ok
R1 | src/sync/timeout_tests.rs:215 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:216 | test | `expect("db dir")` panics on failure
R1 | src/sync/timeout_tests.rs:217 | test | `expect("open db")` panics on failure
R1 | src/sync/timeout_tests.rs:226 | test | `expect("seed publish")` panics on failure
R1 | src/sync/timeout_tests.rs:232 | test | `expect("seeded manifest hash")` panics on failure
R1 | src/sync/timeout_tests.rs:240 | test | `expect("exclude-branch manifest hash")` panics on failure
R1 | src/sync/timeout_tests.rs:253 | test | `expect("project dir")` panics on failure
R1 | src/sync/timeout_tests.rs:259 | test | `expect_err("a spent budget must abort discovery mid-walk")` panics if the result is Ok
R2 | src/sync/sample_batch.rs:297 | prod | when the error slot is already filled, a later worker's PublishError is discarded with no log; only the first worker's failure reaches the caller, and the doc's first-wins rule never surfaces the second distinct error
R5 | src/sync/publish_cas.rs:73 | prod | `PublishError::InvalidParserPackVersion` is returned here, but no test drives an invalid pack version through `publish`/`publish_with_limits`; manager_meta_tests.rs:277 only constructs the value to exercise the error-code mapper
R5 | src/sync/publish_attempt.rs:146 | prod | `PublishError::Database` is returned here (and by the `?` conversions in publish_diff/manifest); `manager_observability_tests.rs:128,150` do inject a mid-publish rusqlite failure via the race hook, but both assert only `is_err()`, which R5 does not count — no test asserts the exact `Database` variant
R7 | src/sync/manifest.rs:113 | prod | `paths_json_array` serializes the whole upsert+deletion+skipped path set into one JSON SQL parameter with no count/size cap and no deadline check; `max_total_import_bytes` bounds file bytes but not the number of (e.g. empty) files, so a tree of millions of tiny files builds an unbounded allocation and SQL value
R8 | src/sync/manifest.rs:17 | prod | `ChangeStatus::Added` variant has no doc comment (type re-exported at src/sync/mod.rs:33)
R8 | src/sync/manifest.rs:18 | prod | `ChangeStatus::Modified` variant has no doc comment
R8 | src/sync/manifest.rs:19 | prod | `ChangeStatus::Deleted` variant has no doc comment
R8 | src/sync/manifest.rs:25 | prod | `FileChange.relative_path` public field has no doc comment
R8 | src/sync/manifest.rs:26 | prod | `FileChange.status` public field has no doc comment
R8 | src/sync/publish.rs:34 | prod | `PublishError::Discovery` variant has no doc comment (enum re-exported at src/sync/mod.rs:34)
R8 | src/sync/publish.rs:36 | prod | `PublishError::Sample` variant has no doc comment
R8 | src/sync/publish.rs:38 | prod | `PublishError::Revision` variant has no doc comment
R8 | src/sync/publish.rs:40 | prod | `PublishError::Database` variant has no doc comment
R8 | src/sync/publish.rs:44 | prod | `PublishError::ImportTooLarge` variant (col 5) has no doc comment
R8 | src/sync/publish.rs:44 | prod | `ImportTooLarge.total` public field (col 22) has no doc comment
R8 | src/sync/publish.rs:44 | prod | `ImportTooLarge.limit` public field (col 34) has no doc comment
R8 | src/sync/publish.rs:53 | prod | `ChangedDuringSample.path` public field (col 27) has no doc comment
R8 | src/sync/publish.rs:58 | prod | `SamplingResultLost.path` public field (col 26) has no doc comment
R8 | src/sync/publish.rs:58 | prod | `SamplingResultLost.reason` public field (col 40) has no doc comment
R8 | src/sync/publish.rs:60 | prod | `PublishError::InvalidParserPackVersion` variant has no doc comment
R8 | src/sync/publish.rs:67 | prod | `TimeBudgetExceeded.path` public field (col 26) has no doc comment
R8 | src/sync/publish.rs:67 | prod | `TimeBudgetExceeded.elapsed_ms` public field (col 40) has no doc comment
R8 | src/sync/publish.rs:91 | prod | `PublishReport.revision_id` public field has no doc comment
R8 | src/sync/publish.rs:98 | prod | `PublishReport.modified` public field has no doc comment
R8 | src/sync/publish.rs:99 | prod | `PublishReport.deleted` public field has no doc comment
R8 | src/sync/publish.rs:100 | prod | `PublishReport.unchanged` public field has no doc comment
R10 | src/sync/publish_race_tests.rs:44 | test | identical 10-line sample+diff setup repeated at src/sync/publish_race_tests.rs:73-82 (could be one helper)
R10 | src/sync/publish_mutation_tests.rs:100 | test | identical 14-line bootstrap-publish block at src/sync/publish_race_tests.rs:150-163 (and again at publish_race_tests.rs:225-238)
R10 | src/sync/sample_tests.rs:169 | test | identical 10-line `Sample` fixture construction repeated at src/sync/sample_tests.rs:190-199
R10 | src/sync/publish_mutation_tests.rs:209 | test | identical 11-line oversized-file fixture repeated at src/sync/publish_mutation_tests.rs:254-264
R10 | src/sync/publish_race_tests.rs:165 | test | identical 10-line barrier/channel/spawn setup repeated at src/sync/publish_race_tests.rs:240-249
R12 | src/sync/publish_log.rs:16 | prod | `error = %error` puts raw absolute paths into a correlation-bearing sync log: `SampleError::Read`/`TooLarge` and `DiscoveryError::Read` embed `PathBuf` in their Display, bypassing the `*_id = redacted_path(...)` rule; this is not a rule-(b) one-shot rejection/cleanup diagnostic
R12 | src/sync/publish_log.rs:16 | prod | double log of one terminal failure: the same error is logged again by `manager_meta::map_publish_error` (src/sync/manager_meta.rs:233 — the ledger said :232) on the `publish_full` path, so every failed publish emits two warn lines
R12 | src/sync/publish_cas.rs:96 | prod | `reason = %error` on each retry embeds `ChangedDuringSample.path` (project-relative) in a correlation-bearing sync log without `redacted_path`; not covered by the rule-(b) one-shot exception or rule-(c) ignore-matcher exception
R12 | src/sync/publish_attempt.rs:57 | prod | `reason = %file.reason` logs the full skip message; for sampling skips that string is `SampleError::TooLarge`'s Display, which embeds the raw absolute path, defeating the redacted `path_id` on the preceding line

## Chunk 7

R1 | src/sync/race_hook.rs:54 | test | `.expect("real root")` panics on failure.
R1 | src/sync/race_hook.rs:55 | test | `.expect("link dir")` panics on failure.
R1 | src/sync/race_hook.rs:57 | test | `.expect("symlink root")` panics on failure.
R1 | src/sync/reconcile/queries.rs:71 | test | `.expect("db dir")` panics on failure.
R1 | src/sync/reconcile/queries.rs:73 | test | `.expect("open db")` panics on failure.
R1 | src/sync/reconcile/queries.rs:75 | test | `.expect("query version")` panics on failure.
R1 | src/sync/reconcile/queries.rs:87 | test | `.expect("40k-path lookup")` panics on failure.
R1 | src/sync/reconcile/queries.rs:103 | test | `.expect("current revision id")` panics on failure.
R1 | src/sync/reconcile/queries.rs:116 | test | `.expect("insert stored hash fixture")` panics on failure.
R1 | src/sync/reconcile/queries.rs:121 | test | `.expect("batch lookup")` panics on failure.
R1 | src/sync/reconcile/reconcile_binary_tests.rs:32 | test | `.expect("publish with binary file")` panics on failure.
R1 | src/sync/reconcile/reconcile_binary_tests.rs:39 | test | `.expect("read binary row")` panics on failure.
R1 | src/sync/reconcile/reconcile_binary_tests.rs:58 | test | `.expect("reconcile modified binary")` panics on failure.
R1 | src/sync/reconcile/reconcile_binary_tests.rs:75 | test | `.expect("read binary row")` panics on failure.
R1 | src/sync/reconcile/reconcile_ignore_tests.rs:44 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_ignore_tests.rs:62 | test | `.expect("count")` panics on failure.
R1 | src/sync/reconcile/reconcile_ignore_tests.rs:73 | test | `.expect("count")` panics on failure.
R1 | src/sync/reconcile/reconcile_ignore_tests.rs:95 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_ignore_tests.rs:104 | test | `.expect("count")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:25 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:49 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:76 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:89 | test | `.expect("count")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:107 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:119 | test | `.expect("count")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:143 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:155 | test | `.expect("remove file")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:167 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:192 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:209 | test | `.expect("barrier sync should succeed")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:230 | test | `.expect("barrier sync should succeed")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:235 | test | `.expect("paths received")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:257 | test | `.expect("barrier sync should succeed")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:289 | test | `.expect_err("the loop must hit the cap")` panics if the call unexpectedly succeeds.
R1 | src/sync/reconcile/reconcile_tests.rs:294 | test | `panic!("expected BarrierCapExceeded, got {other:?}")` panics on an unexpected error variant.
R1 | src/sync/reconcile/reconcile_tests.rs:330 | test | `.expect_err("a mismatched expected revision must be rejected")` panics if the call unexpectedly succeeds.
R1 | src/sync/reconcile/reconcile_tests.rs:371 | test | `.expect("an oversized dirty path must be skipped, not fatal")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:420 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:430 | test | `.expect("main.py's edge")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:443 | test | `.expect("create dir")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:454 | test | `.expect("a dirty directory must not fail the reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:466 | test | `.expect("outside dir")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:467 | test | `.expect("write target")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:472 | test | `.expect("create symlink")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:482 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:491 | test | `.expect("count link rows")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:504 | test | `.expect("count revisions")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:511 | test | `.expect("stored hash")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:528 | test | `.expect_err("a spent budget must abort before any lookup")` panics if the call unexpectedly succeeds.
R1 | src/sync/reconcile/reconcile_tests.rs:536 | test | `.expect("count revisions")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:543 | test | `.expect("stored hash")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:559 | test | `.expect("remove file")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:560 | test | `.expect("create dir in its place")` panics on failure.
R1 | src/sync/reconcile/reconcile_tests.rs:570 | test | `.expect("reconcile")` panics on failure.
R1 | src/sync/revision_tests.rs:19 | test | `.expect("insert finding")` panics on failure.
R1 | src/sync/revision_tests.rs:25 | test | `.expect("prepare")` panics on failure.
R1 | src/sync/revision_tests.rs:27 | test | `.expect("query")` panics on failure.
R1 | src/sync/revision_tests.rs:29 | test | `.expect("rows")` panics on failure.
R1 | src/sync/revision_tests.rs:34 | test | `.expect("project dir")` panics on failure.
R1 | src/sync/revision_tests.rs:35 | test | `.expect("db dir")` panics on failure.
R1 | src/sync/revision_tests.rs:36 | test | `.expect("open db")` panics on failure.
R1 | src/sync/revision_tests.rs:40 | test | `.expect("seed publish")` panics on failure.
R1 | src/sync/revision_tests.rs:47 | test | `.expect("stored hash")` panics on failure.
R1 | src/sync/revision_tests.rs:57 | test | `.expect("transaction")` panics on failure.
R1 | src/sync/revision_tests.rs:59 | test | `.expect("invalidate with 40k touched paths")` panics on failure.
R1 | src/sync/revision_tests.rs:60 | test | `.expect("commit")` panics on failure.
R1 | src/sync/revision_tests.rs:74 | test | `.expect("project dir")` panics on failure.
R1 | src/sync/revision_tests.rs:75 | test | `.expect("db dir")` panics on failure.
R1 | src/sync/revision_tests.rs:76 | test | `.expect("open db")` panics on failure.
R1 | src/sync/revision_tests.rs:79 | test | `.expect("seed publish")` panics on failure.
R1 | src/sync/revision_tests.rs:85 | test | `.expect("hash with 40k unrelated deletions")` panics on failure.
R1 | src/sync/revision_tests.rs:90 | test | `.expect("hash with 40k+1 deletions")` panics on failure.
R1 | src/sync/revision_tests.rs:102 | test | `.expect("system clock is after the epoch")` panics on failure.
R1 | src/sync/revision_tests.rs:107 | test | `.expect("system clock is after the epoch")` panics on failure.
R1 | src/sync/revision_tests.rs:110 | test | `.expect("a real current timestamp is never negative")` panics on failure.
R1 | src/sync/revision_tests.rs:120 | test | `.expect("project dir")` panics on failure.
R1 | src/sync/revision_tests.rs:121 | test | `.expect("db dir")` panics on failure.
R1 | src/sync/revision_tests.rs:122 | test | `.expect("open db")` panics on failure.
R1 | src/sync/revision_tests.rs:129 | test | `.expect("a first publish touches every file and must not hit the bound-variable limit")` panics on failure.
R1 | src/sync/revision_tests.rs:133 | test | `.expect("count files")` panics on failure.
R5 | src/sync/revision.rs:34 | prod | Public `RevisionError::Database` variant has no test that drives a database failure through `publish_revision` (or any caller) and asserts this exact variant; every `?` in the publish path maps to it untested.
R5 | src/sync/revision_files.rs:34 | prod | `RevisionError::InvalidParserRun` mapping from `file.run.validate()` is untested: `publish_mutation_tests.rs:27` constructs the variant directly only to check `is_retryable`, never driving `upsert_file`/`publish_revision` with an invalid parser run and asserting this error.
R8 | src/sync/revision.rs:14 | prod | Public `FileRecord::relative_path` has no doc comment (authoritative `-W missing_docs` list).
R8 | src/sync/revision.rs:15 | prod | Public `FileRecord::is_binary` has no doc comment.
R8 | src/sync/revision.rs:16 | prod | Public `FileRecord::content` has no doc comment.
R8 | src/sync/revision.rs:17 | prod | Public `FileRecord::identity` has no doc comment.
R8 | src/sync/revision.rs:18 | prod | Public `FileRecord::byte_len` has no doc comment.
R8 | src/sync/revision.rs:23 | prod | Public `FileRecord::language` has no doc comment.
R8 | src/sync/revision.rs:24 | prod | Public `FileRecord::language_detected` has no doc comment.
R8 | src/sync/revision.rs:25 | prod | Public `FileRecord::run` has no doc comment.
R8 | src/sync/revision.rs:26 | prod | Public `FileRecord::evidence` has no doc comment.
R8 | src/sync/revision.rs:34 | prod | Public `RevisionError::Database` variant has no doc comment.
R8 | src/sync/revision.rs:36 | prod | Public `RevisionError::InvalidParserRun` variant has no doc comment.
R8 | src/sync/revision.rs:36 | prod | Public `RevisionError::InvalidParserRun` field `path` has no doc comment.
R8 | src/sync/revision.rs:36 | prod | Public `RevisionError::InvalidParserRun` field `reason` has no doc comment.
R8 | src/sync/revision.rs:43 | prod | Public `RevisionError::StaleBaseline` field `expected` has no doc comment.
R8 | src/sync/revision.rs:43 | prod | Public `RevisionError::StaleBaseline` field `found` has no doc comment.
R10 | src/sync/reconcile/reconcile_tests.rs:11 | test | `unchanged_dirty_paths_are_skipped` (lines 10-30) duplicates the body of `no_changes_produces_no_revision` (lines 176-197): identical setup, dirty/deleted construction, 8-line `reconcile_dirty_paths` call, and assertions; could share one helper or one test.
R12 | src/sync/reconcile/barrier.rs:81 | prod | Double log: the barrier-cap failure is logged at warn here, then the same `ReconcileError::BarrierCapExceeded` propagates and is logged a second time at `src/sync/manager_health.rs:192` (healthy path) or `src/sync/manager_health.rs:95` (post-verification drain) — two warn lines for one failure.

## Chunk 8

R1 | src/tools/mod.rs:209 | test | `expect("temp dir")` panics on failure
R1 | src/tools/mod.rs:211 | test | `expect("utf-8 path")` panics on failure
R1 | src/tools/mod.rs:212 | test | `expect_err("a path with no project root must fail, not fall back")` panics if the call succeeds
R1 | src/tools/mod.rs:227 | test | `expect("utf-8 path")` panics on failure
R1 | src/tools/mod.rs:228 | test | `expect("an active project resolves")` panics on failure
R1 | src/tools/mod.rs:237 | test | `expect("in-memory database")` panics on failure
R1 | src/tools/mod.rs:246 | test | `expect("evaluate credential predicate")` panics on failure
R1 | src/tools/context_race_tests.rs:16 | test | `expect("project dir")` panics on failure
R1 | src/tools/context_race_tests.rs:18 | test | `expect("activate project")` panics on failure
R1 | src/tools/context_race_tests.rs:19 | test | `expect("write fixture file")` panics on failure
R1 | src/tools/context_race_tests.rs:40 | test | `expect("initial sync")` panics on failure
R1 | src/tools/context_race_tests.rs:62 | test | `expect("signal reader parked")` panics on failure
R1 | src/tools/context_race_tests.rs:63 | test | `expect("wait for the publish to land")` panics on failure
R1 | src/tools/context_race_tests.rs:84 | test | `expect("reader reached its parked midpoint")` panics on failure
R1 | src/tools/context_race_tests.rs:92 | test | `expect("modify file")` panics on failure
R1 | src/tools/context_race_tests.rs:93 | test | `expect("add file")` panics on failure
R1 | src/tools/context_race_tests.rs:94 | test | `expect("open db (writer)")` panics on failure
R1 | src/tools/context_race_tests.rs:101 | test | `expect("concurrent publish succeeds")` panics on failure
R1 | src/tools/context_race_tests.rs:108 | test | `expect("release the parked reader")` panics on failure
R1 | src/tools/context_race_tests.rs:111 | test | `expect("reader thread joins")` panics on failure
R1 | src/tools/context_race_tests.rs:112 | test | `expect("the paused read still succeeds despite the concurrent publish")` panics on failure
R1 | src/tools/context_race_tests.rs:132 | test | `expect("re-sync")` panics on failure
R1 | src/tools/context_race_tests.rs:137 | test | `expect("fresh read succeeds")` panics on failure
R1 | src/tools/context_tests.rs:10 | test | `expect("initial sync")` panics on failure
R1 | src/tools/context_tests.rs:13 | test | `expect("corrupt derived database")` panics on failure
R1 | src/tools/context_tests.rs:18 | test | `expect("write stale wal sidecar")` panics on failure
R1 | src/tools/context_tests.rs:21 | test | `expect("corrupt cache is rebuilt")` panics on failure
R1 | src/tools/context_tests.rs:24 | test | `expect("rebuilt db")` panics on failure
R1 | src/tools/context_tests.rs:27 | test | `expect("rebuilt file row")` panics on failure
R1 | src/tools/context_tests.rs:42 | test | `expect("local sync")` panics on failure
R1 | src/tools/context_tests.rs:56 | test | `expect("sync")` panics on failure
R1 | src/tools/context_tests.rs:62 | test | `expect("still-fresh revision opens fine and the closure's query runs")` panics on failure
R1 | src/tools/context_tests.rs:78 | test | `expect("modify file")` panics on failure
R1 | src/tools/context_tests.rs:90 | test | `expect_err("a stale revision handle must never open against newer data")` panics if the call succeeds
R1 | src/tools/context_tests.rs:108 | test | `expect("a stale revision carries a payload")` panics on failure
R1 | src/tools/context_tests.rs:110 | test | `expect("remedy is a string")` panics on failure
R1 | src/tools/context_tests.rs:122 | test | `expect("first sync establishes the project row")` panics on failure
R1 | src/tools/context_tests.rs:132 | test | `expect("open raw connection")` panics on failure
R1 | src/tools/context_tests.rs:137 | test | `expect("simulate a copied database from another project")` panics on failure
R1 | src/tools/context_tests.rs:145 | test | `expect("a stale root must be rebuilt, not served or rejected")` panics on failure
R1 | src/tools/context_tests.rs:147 | test | `expect("rebuilt db")` panics on failure
R1 | src/tools/context_tests.rs:152 | test | `expect("read stored root")` panics on failure
R1 | src/tools/context_tests.rs:159 | test | `expect("rebuilt file row")` panics on failure
R1 | src/tools/context_tests.rs:176 | test | `expect("modify file")` panics on failure
R1 | src/tools/context_tests.rs:189 | test | `expect_err("a stale write handle must be rejected")` panics if the call succeeds
R1 | src/tools/context_tests.rs:202 | test | `expect("fresh handle reads fine")` panics on failure
R1 | src/tools/context_tests.rs:219 | test | `expect("sync")` panics on failure
R1 | src/tools/context_tests.rs:231 | test | `expect("write against a current revision succeeds")` panics on failure
R1 | src/tools/context_tests.rs:237 | test | `expect("read back the committed row")` panics on failure
R1 | src/tools/context_tests.rs:249 | test | `expect("first sync")` panics on failure
R1 | src/tools/context_tests.rs:251 | test | `expect("second sync")` panics on failure
R1 | src/tools/context_transaction_tests.rs:20 | test | `expect_err("opening a missing database must fail")` panics if the call succeeds
R1 | src/tools/context_transaction_tests.rs:39 | test | `expect("db dir")` panics on failure
R1 | src/tools/context_transaction_tests.rs:41 | test | `expect("write corrupt db")` panics on failure
R1 | src/tools/finding_race_tests.rs:15 | test | `expect("project dir")` panics on failure
R1 | src/tools/finding_race_tests.rs:17 | test | `expect("activate project")` panics on failure
R1 | src/tools/finding_race_tests.rs:18 | test | `expect("write fixture file")` panics on failure
R1 | src/tools/finding_race_tests.rs:40 | test | `expect("re-sync")` panics on failure
R1 | src/tools/finding_race_tests.rs:45 | test | `expect("read finding count")` panics on failure
R1 | src/tools/finding_race_tests.rs:62 | test | `expect("initial sync")` panics on failure
R1 | src/tools/finding_race_tests.rs:72 | test | `expect("signal the revision was captured")` panics on failure
R1 | src/tools/finding_race_tests.rs:75 | test | `expect("wait for the concurrent publish to land")` panics on failure
R1 | src/tools/finding_race_tests.rs:83 | test | `expect("writer thread captured its revision")` panics on failure
R1 | src/tools/finding_race_tests.rs:89 | test | `expect("modify file")` panics on failure
R1 | src/tools/finding_race_tests.rs:90 | test | `expect("open db (writer)")` panics on failure
R1 | src/tools/finding_race_tests.rs:97 | test | `expect("concurrent publish succeeds")` panics on failure
R1 | src/tools/finding_race_tests.rs:104 | test | `expect("let the writer proceed")` panics on failure
R1 | src/tools/finding_race_tests.rs:105 | test | `expect("writer thread joins")` panics on failure
R1 | src/tools/finding_race_tests.rs:107 | test | `expect_err` panics if the call succeeds
R1 | src/tools/finding_race_tests.rs:135 | test | `expect("initial sync")` panics on failure
R1 | src/tools/finding_race_tests.rs:151 | test | `expect("add unrelated file")` panics on failure
R1 | src/tools/finding_race_tests.rs:154 | test | `expect("open db (publisher)")` panics on failure
R1 | src/tools/finding_race_tests.rs:163 | test | `expect("writer thread joins")` panics on failure
R1 | src/tools/finding_race_tests.rs:164 | test | `expect("publisher thread joins")` panics on failure
R1 | src/tools/finding_race_tests.rs:165 | test | `expect("publish always converges through its own CAS retry loop")` panics on failure
R1 | src/tools/finding_read_tests.rs:60 | test | `expect("write finding")` panics on failure
R1 | src/tools/finding_read_tests.rs:75 | test | `expect("finding_read")` panics on failure
R1 | src/tools/finding_read_tests.rs:91 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_read_tests.rs:94 | test | `expect("count")` panics on failure
R1 | src/tools/finding_read_tests.rs:102 | test | `expect("finding_read")` panics on failure
R1 | src/tools/finding_read_tests.rs:112 | test | `expect("b.rs")` panics on failure
R1 | src/tools/finding_read_tests.rs:117 | test | `expect("initial sync")` panics on failure
R1 | src/tools/finding_read_tests.rs:122 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_read_tests.rs:125 | test | `expect("count")` panics on failure
R1 | src/tools/finding_read_tests.rs:133 | test | `expect("filtered")` panics on failure
R1 | src/tools/finding_read_tests.rs:152 | test | `expect("before")` panics on failure
R1 | src/tools/finding_read_tests.rs:156 | test | `expect("modify")` panics on failure
R1 | src/tools/finding_read_tests.rs:157 | test | `expect("re-sync")` panics on failure
R1 | src/tools/finding_read_tests.rs:164 | test | `expect("after")` panics on failure
R1 | src/tools/finding_session_tests.rs:46 | test | `expect("open read-only")` panics on failure
R1 | src/tools/finding_session_tests.rs:52 | test | `expect("count findings")` panics on failure
R1 | src/tools/finding_session_tests.rs:69 | test | `expect("finding succeeds")` panics on failure
R1 | src/tools/finding_session_tests.rs:82 | test | `expect("read session_id")` panics on failure
R1 | src/tools/finding_session_tests.rs:85 | test | `expect("stamped session id parses as uuid")` panics on failure
R1 | src/tools/finding_session_tests.rs:115 | test | `expect("first-session finding succeeds")` panics on failure
R1 | src/tools/finding_session_tests.rs:146 | test | `expect("report under new session wipes and reports")` panics on failure
R1 | src/tools/finding_session_tests.rs:159 | test | `expect("second-session finding succeeds")` panics on failure
R1 | src/tools/finding_session_tests.rs:172 | test | `expect("read next session_id")` panics on failure
R1 | src/tools/finding_session_tests.rs:209 | test | `expect("first-session finding lands")` panics on failure
R1 | src/tools/finding_session_tests.rs:222 | test | `expect("modify file")` panics on failure
R1 | src/tools/finding_session_tests.rs:239 | test | `expect("resync under new session")` panics on failure
R1 | src/tools/finding_session_tests.rs:263 | test | `expect("initial sync")` panics on failure
R1 | src/tools/finding_session_tests.rs:270 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_session_tests.rs:277 | test | `expect("read the indexed content hash")` panics on failure
R1 | src/tools/finding_session_tests.rs:288 | test | `expect("seed a foreign-session row with the same identity fields")` panics on failure
R1 | src/tools/finding_session_tests.rs:294 | test | `expect("begin write transaction")` panics on failure
R1 | src/tools/finding_session_tests.rs:295 | test | `expect("finding succeeds")` panics on failure
R1 | src/tools/finding_session_tests.rs:296 | test | `expect("commit")` panics on failure
R1 | src/tools/finding_session_tests.rs:304 | test | `expect("prepare session read")` panics on failure
R1 | src/tools/finding_session_tests.rs:306 | test | `expect("query sessions")` panics on failure
R1 | src/tools/finding_session_tests.rs:308 | test | `expect("collect sessions")` panics on failure
R1 | src/tools/finding_tests.rs:27 | test | `expect("read finding status")` panics on failure
R1 | src/tools/finding_tests.rs:38 | test | `expect("finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:53 | test | `expect("finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:60 | test | `expect("modify file")` panics on failure
R1 | src/tools/finding_tests.rs:66 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_tests.rs:73 | test | `expect("resync")` panics on failure
R1 | src/tools/finding_tests.rs:86 | test | `expect("finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:89 | test | `expect("delete file")` panics on failure
R1 | src/tools/finding_tests.rs:95 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_tests.rs:102 | test | `expect("resync")` panics on failure
R1 | src/tools/finding_tests.rs:110 | test | `expect("write second file")` panics on failure
R1 | src/tools/finding_tests.rs:116 | test | `expect("finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:123 | test | `expect("modify other file")` panics on failure
R1 | src/tools/finding_tests.rs:129 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_tests.rs:136 | test | `expect("resync")` panics on failure
R1 | src/tools/finding_tests.rs:149 | test | `expect("open db")` panics on failure
R1 | src/tools/finding_tests.rs:156 | test | `expect("sync")` panics on failure
R1 | src/tools/finding_tests.rs:160 | test | `expect("count findings")` panics on failure
R1 | src/tools/finding_tests.rs:174 | test | explicit `panic!("empty title must error")` in test
R1 | src/tools/finding_tests.rs:191 | test | explicit `panic!("reversed line range must error")` in test
R1 | src/tools/finding_tests.rs:206 | test | explicit `panic!("unindexed file must error")` in test
R1 | src/tools/finding_tests.rs:223 | test | explicit `panic!("line number past length must error")` in test
R1 | src/tools/finding_tests.rs:255 | test | explicit `panic!("oversized description must error")` in test
R1 | src/tools/finding_tests.rs:269 | test | explicit `panic!("empty file finding must error")` in test
R1 | src/tools/finding_tests.rs:291 | test | `expect("first finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:301 | test | `expect("restated finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:313 | test | `expect("open db read-only")` panics on failure
R1 | src/tools/finding_tests.rs:316 | test | `expect("count findings")` panics on failure
R1 | src/tools/finding_tests.rs:324 | test | `expect("read refreshed row")` panics on failure
R1 | src/tools/finding_tests.rs:346 | test | `expect("first finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:356 | test | `expect("second finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:367 | test | `expect("open db read-only")` panics on failure
R1 | src/tools/finding_tests.rs:370 | test | `expect("count findings")` panics on failure
R1 | src/tools/finding_tests.rs:388 | test | `expect("first finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:394 | test | `expect("modify file")` panics on failure
R1 | src/tools/finding_tests.rs:400 | test | `expect("restated finding succeeds")` panics on failure
R1 | src/tools/finding_tests.rs:411 | test | `expect("open db read-only")` panics on failure
R1 | src/tools/finding_tests.rs:414 | test | `expect("count findings")` panics on failure
R1 | src/tools/finding_tests.rs:423 | test | `expect("binary file")` panics on failure
R1 | src/tools/finding_tests.rs:433 | test | explicit `panic!("a binary file has no indexed content")` in test
R1 | src/tools/finding_tests.rs:435 | test | `expect("a binary file carries a payload")` panics on failure
R1 | src/tools/health_state_tests.rs:14 | test | `expect("project dir")` panics on failure
R1 | src/tools/health_state_tests.rs:15 | test | `expect("activation dir")` panics on failure
R1 | src/tools/health_state_tests.rs:16 | test | `expect("write source")` panics on failure
R1 | src/tools/health_state_tests.rs:25 | test | `expect("sync")` panics on failure
R1 | src/tools/health_state_tests.rs:41 | test | `expect("project state")` panics on failure
R1 | src/tools/health_state_tests.rs:43 | test | `expect("watcher group")` panics on failure
R1 | src/tools/health_state_tests.rs:48 | test | `expect("database group")` panics on failure
R1 | src/tools/health_state_tests.rs:80 | test | `expect("watcher group")` panics on failure
R1 | src/tools/health_state_tests.rs:93 | test | `expect("open")` panics on failure
R1 | src/tools/health_state_tests.rs:96 | test | `expect("read revision fields")` panics on failure
R1 | src/tools/health_state_tests.rs:100 | test | `expect("file count")` panics on failure
R1 | src/tools/health_state_tests.rs:109 | test | `expect("in-memory db")` panics on failure
R1 | src/tools/health_state_tests.rs:111 | test | `expect_err("a missing revisions table must error, not read as no revision")` panics if the call succeeds
R1 | src/tools/health_state_tests.rs:120 | test | `expect("db dir")` panics on failure
R1 | src/tools/health_state_tests.rs:122 | test | `expect("open fresh db")` panics on failure
R1 | src/tools/health_state_tests.rs:124 | test | `expect("read revision fields")` panics on failure
R1 | src/tools/health_state_tests.rs:128 | test | `expect("query")` panics on failure
R1 | src/tools/health_state_tests.rs:137 | test | `expect("project dir")` panics on failure
R1 | src/tools/health_state_tests.rs:138 | test | `expect("activation dir")` panics on failure
R1 | src/tools/health_state_tests.rs:139 | test | `expect("write source")` panics on failure
R1 | src/tools/health_state_tests.rs:143 | test | `expect("no error")` panics on failure
R1 | src/tools/health_state_tests.rs:160 | test | `expect("initial sync")` panics on failure
R1 | src/tools/health_state_tests.rs:169 | test | `expect("modify source")` panics on failure
R1 | src/tools/health_state_tests.rs:171 | test | `expect("health read")` panics on failure
R1 | src/tools/health_state_tests.rs:174 | test | `expect("open")` panics on failure
R1 | src/tools/health_state_tests.rs:177 | test | `expect("revision count")` panics on failure
R1 | src/tools/health_state_tests.rs:202 | test | `expect("ensure_current succeeds via the test manager")` panics on failure
R1 | src/tools/health_state_tests.rs:210 | test | `expect("health with a path succeeds")` panics on failure
R1 | src/tools/health_state_tests.rs:213 | test | `expect("database group")` panics on failure
R1 | src/tools/health_state_tests.rs:231 | test | `expect("project dir")` panics on failure
R1 | src/tools/health_state_tests.rs:232 | test | `expect("write source")` panics on failure
R1 | src/tools/health_state_tests.rs:240 | test | `expect("a disabled path is an answer, not an error")` panics on failure
R1 | src/tools/health_state_tests.rs:249 | test | `expect("a disabled answer carries the remedy")` panics on failure
R1 | src/tools/health_state_tests.rs:264 | test | `expect("health never fails without a path")` panics on failure
R1 | src/tools/health_state_tests.rs:282 | test | `expect("health succeeds for an active project")` panics on failure
R1 | src/tools/health_state_tests.rs:290 | test | `expect("database group")` panics on failure
R1 | src/tools/health_state_tests.rs:303 | test | `expect("read existing published database")` panics on failure
R1 | src/tools/health_state_tests.rs:306 | test | `expect("database group")` panics on failure
R1 | src/tools/health_state_tests.rs:321 | test | `expect("dir")` panics on failure
R1 | src/tools/health_state_tests.rs:323 | test | `expect("write file")` panics on failure
R1 | src/tools/health_tests.rs:171 | test | `expect("health should not fail on shape")` panics on failure
R1 | src/tools/health_tests.rs:205 | test | `expect("temp dir")` panics on failure
R1 | src/tools/health_tests.rs:207 | test | `expect("create the project database")` panics on failure
R1 | src/tools/health_tests.rs:214 | test | `expect("widen database permissions")` panics on failure
R1 | src/tools/health_tests.rs:224 | test | `expect("temp dir")` panics on failure
R1 | src/tools/project_control_tests.rs:10 | test | `expect("project dir")` panics on failure
R1 | src/tools/project_control_tests.rs:11 | test | `expect("create src dir")` panics on failure
R1 | src/tools/project_control_tests.rs:12 | test | `expect("write source")` panics on failure
R1 | src/tools/project_control_tests.rs:40 | test | `expect("request deserializes")` panics on failure
R1 | src/tools/project_control_tests.rs:47 | test | `expect("request deserializes")` panics on failure
R1 | src/tools/project_control_tests.rs:54 | test | `expect("request deserializes")` panics on failure
R1 | src/tools/project_control_tests.rs:62 | test | `expect("enable succeeds")` panics on failure
R1 | src/tools/project_control_tests.rs:71 | test | `expect("tempdir canonicalizes")` panics on failure
R1 | src/tools/project_control_tests.rs:75 | test | `expect("import report present")` panics on failure
R1 | src/tools/project_control_tests.rs:96 | test | `expect("first enable")` panics on failure
R1 | src/tools/project_control_tests.rs:98 | test | `expect("second enable")` panics on failure
R1 | src/tools/project_control_tests.rs:109 | test | `expect("enable")` panics on failure
R1 | src/tools/project_control_tests.rs:117 | test | `expect("disable succeeds")` panics on failure
R1 | src/tools/project_control_tests.rs:138 | test | `expect("enable")` panics on failure
R1 | src/tools/project_control_tests.rs:144 | test | `expect("disable")` panics on failure
R1 | src/tools/project_control_tests.rs:160 | test | `expect("noop disable")` panics on failure
R1 | src/tools/project_control_tests.rs:189 | test | `expect("enable with profile succeeds")` panics on failure
R1 | src/tools/project_control_tests.rs:194 | test | `expect("read config")` panics on failure
R1 | src/tools/project_control_tests.rs:202 | test | `expect("create config parent")` panics on failure
R1 | src/tools/project_control_tests.rs:227 | test | `expect("enable succeeds")` panics on failure
R1 | src/tools/project_control_tests.rs:232 | test | `expect("enable reports the gitignore guard")` panics on failure
R1 | src/tools/project_control_tests.rs:236 | test | `expect("tempdir canonicalizes")` panics on failure
R1 | src/tools/project_control_tests.rs:246 | test | `expect("read .gitignore")` panics on failure
R1 | src/tools/project_control_tests.rs:257 | test | `expect("write .gitignore")` panics on failure
R1 | src/tools/project_control_tests.rs:260 | test | `expect("enable succeeds")` panics on failure
R1 | src/tools/project_control_tests.rs:265 | test | `expect("enable reports the gitignore guard")` panics on failure
R1 | src/tools/project_control_tests.rs:270 | test | `expect("read .gitignore")` panics on failure
R1 | src/tools/project_control_tests.rs:279 | test | `expect("block the ignore file")` panics on failure
R1 | src/tools/project_control_tests.rs:282 | test | `expect("the convenience guard must not fail the enable")` panics on failure
R1 | src/tools/project_control_tests.rs:287 | test | `expect("enable reports the gitignore guard")` panics on failure
R1 | src/tools/project_control_tests.rs:300 | test | `expect("enable")` panics on failure
R1 | src/tools/project_control_tests.rs:302 | test | `expect("read .gitignore")` panics on failure
R1 | src/tools/project_control_tests.rs:309 | test | `expect("disable succeeds")` panics on failure
R1 | src/tools/project_control_tests.rs:317 | test | `expect("read .gitignore")` panics on failure
R1 | src/tools/project_control_tests.rs:334 | test | `expect("a profile that does not exist must not enable on the default")` panics on failure
R1 | src/tools/project_control_tests.rs:351 | test | `expect("write file")` panics on failure
R1 | src/tools/project_control_tests.rs:359 | test | `expect("an uninspectable activation dir is not 'already disabled'")` panics on failure
R1 | src/tools/page_size.rs:55 | test | `expect("default resolves")` panics on failure
R1 | src/tools/page_size.rs:68 | test | `expect("clamps")` panics on failure
R1 | src/tools/page_size.rs:81 | test | `expect("clamps")` panics on failure
R1 | src/tools/page_size.rs:88 | test | `expect_err("zero is invalid")` panics if the call succeeds
R1 | src/tools/test_support.rs:10 | test | `expect("project dir")` panics on failure
R1 | src/tools/test_support.rs:12 | test | `expect("activate project")` panics on failure
R1 | src/tools/test_support.rs:13 | test | `expect("write fixture file")` panics on failure
R1 | src/tools/finding_read_tests.rs:153 | test | data-dependent index `findings[0]` with no in-function bounds guard (an empty read panics)
R1 | src/tools/finding_read_tests.rs:165 | test | data-dependent index `findings[0]` with no in-function bounds guard (an empty read panics)
R5 | src/tools/context.rs:88 | prod | reading the current revision fails: the internal_error arm has no test forcing the SELECT to fail (the stale-revision mismatch arm is covered by a_stale_synced_handle_fails_loudly_instead_of_returning_mismatched_data)
R5 | src/tools/context_transactions.rs:11 | prod | read-database open failure: verified_read_fails_cleanly_when_the_database_is_missing asserts only the generic "retry the call" substring, not the "opening the project database" context
R5 | src/tools/context_transactions.rs:14 | prod | read-transaction start failure arm untested
R5 | src/tools/context_transactions.rs:18 | prod | read-transaction commit failure arm untested
R5 | src/tools/context_transactions.rs:27 | prod | write-database open failure: verified_write_fails_cleanly_when_the_database_is_missing asserts only is_err()
R5 | src/tools/context_transactions.rs:30 | prod | write-transaction start failure arm untested
R5 | src/tools/context_transactions.rs:34 | prod | write-transaction commit failure arm untested
R5 | src/tools/finding.rs:128 | prod | restatement SELECT failure (internal_error) untested
R5 | src/tools/finding.rs:135 | prod | restatement UPDATE failure (internal_error) untested
R5 | src/tools/finding.rs:142 | prod | max(created_at_unix) read failure (internal_error) untested
R5 | src/tools/finding.rs:163 | prod | finding INSERT failure (internal_error) untested
R5 | src/tools/finding.rs:187 | prod | path-over-32768-bytes invalid_params arm untested
R5 | src/tools/finding.rs:190 | prod | file-over-8192-bytes invalid_params arm untested
R5 | src/tools/finding.rs:197 | prod | zero line_start/line_end invalid_params arm untested
R5 | src/tools/finding.rs:243 | prod | line-count u32 conversion failure arm untested
R5 | src/tools/finding.rs:234 | prod | query failure arm: a_finding_against_an_unindexed_file_is_a_typed_error accepts the file name, which both of this function's arms contain, so this arm's exact error is not asserted
R5 | src/tools/finding.rs:253 | prod | empty-file arm: an_empty_file_cannot_have_a_finding asserts contains("empty"), which the fixture filename empty.rs satisfies in any message
R5 | src/tools/finding_read.rs:73 | prod | oversized path/file input invalid_params arm untested
R5 | src/tools/finding_read.rs:113 | prod | statement prepare failure (internal_error) untested
R5 | src/tools/finding_read.rs:129 | prod | query_map failure (internal_error) untested
R5 | src/tools/finding_read.rs:133 | prod | row decode failure (internal_error) untested
R5 | src/tools/health.rs:227 | prod | path-over-32768-bytes invalid_params arm untested
R5 | src/tools/health.rs:294 | prod | non-NotActive project resolution failure: health_with_an_unrelated_path_returns_an_error asserts only is_err()
R5 | src/tools/health_database.rs:44 | prod | query_database_state failure arm (warn "failed to read the project database") untested; only the try_exists arm is log-asserted
R5 | src/tools/project_control.rs:126 | prod | path-over-32768-bytes invalid_params arm untested
R5 | src/tools/project_control.rs:133 | prod | profile-over-64-bytes invalid_params arm untested
R5 | src/tools/project_control.rs:153 | prod | ProjectRoot::resolve failure: on_action_with_a_nonexistent_path_returns_an_error asserts only is_err()
R5 | src/tools/project_control.rs:166 | prod | project::enable failure arm untested
R5 | src/tools/project_control.rs:178 | prod | serde_json serialize failure arm untested
R5 | src/tools/project_control.rs:186 | prod | discard_corrupt_database failure arm untested
R5 | src/tools/project_control.rs:188 | prod | reopen-after-corruption failure arm untested
R5 | src/tools/project_control.rs:190 | prod | non-corruption database open failure arm untested
R5 | src/tools/project_control.rs:193 | prod | initial import (sync::publish) failure arm untested
R5 | src/tools/project_control.rs:229 | prod | project::disable failure arm untested
R6 | src/tools/health.rs:373 | prod | the no-path snapshot passes revision_id as &None, so a synced project with a Healthy watcher is reported as phase "importing", documented as "no current revision — the first sync has never succeeded" (health.rs:170-171), a false claim for it
R6 | src/tools/project_control.rs:61 | prod | GitignoreStatus::NotWritten is documented as "the file was left untouched", but ensure_gitignore's create(true)+write_all can create the file and/or leave a partial append before failing (src/project/activation.rs:291-301), so the reported status can be false
R6 | src/tools/finding.rs:234 | prod | any files-table read failure (lock, corruption, schema drift) is returned as invalid_params naming the caller's file, telling the caller its input is invalid when the index query failed
R6 | src/tools/project_control.rs:153 | prod | a nonexistent/unresolvable caller path is wrapped by err() as an internal server error, so the caller is told the server failed rather than that its path argument is wrong and retrying cannot help
R7 | src/tools/finding_read.rs:131 | prod | findings Vec grows with every session-scoped row (description up to 10,000 chars each) with no page/byte cap and no next_offset; a session that writes many findings gets an unbounded response
R8 | src/tools/mod.rs:170 | prod | missing doc comment on pub field ToolCounters::call_count (rustc -W missing_docs)
R8 | src/tools/mod.rs:171 | prod | missing doc comment on pub field ToolCounters::error_count (rustc -W missing_docs)
R8 | src/tools/mod.rs:172 | prod | missing doc comment on pub field ToolCounters::total_ms (rustc -W missing_docs)
R8 | src/tools/finding.rs:69 | prod | missing doc comment on pub field FindingResponse::id (rustc -W missing_docs)
R8 | src/tools/finding.rs:70 | prod | missing doc comment on pub field FindingResponse::revision_id (rustc -W missing_docs)
R8 | src/tools/finding.rs:71 | prod | missing doc comment on pub field FindingResponse::source_hash (rustc -W missing_docs)
R8 | src/tools/finding.rs:72 | prod | missing doc comment on pub field FindingResponse::status (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:37 | prod | missing doc comment on pub field FindingReadEntry::id (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:38 | prod | missing doc comment on pub field FindingReadEntry::path (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:39 | prod | missing doc comment on pub field FindingReadEntry::line_start (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:40 | prod | missing doc comment on pub field FindingReadEntry::line_end (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:41 | prod | missing doc comment on pub field FindingReadEntry::severity (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:42 | prod | missing doc comment on pub field FindingReadEntry::category (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:43 | prod | missing doc comment on pub field FindingReadEntry::title (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:44 | prod | missing doc comment on pub field FindingReadEntry::description (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:45 | prod | missing doc comment on pub field FindingReadEntry::status (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:46 | prod | missing doc comment on pub field FindingReadEntry::source_hash (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:53 | prod | missing doc comment on pub field FindingReadResponse::revision_id (rustc -W missing_docs)
R8 | src/tools/finding_read.rs:54 | prod | missing doc comment on pub field FindingReadResponse::findings (rustc -W missing_docs)
R10 | src/tools/finding_tests.rs:9-20 | test | base_request duplicated verbatim in finding_session_tests.rs:15-26
R10 | src/tools/finding_tests.rs:9-20 | test | base_request duplicated verbatim in finding_race_tests.rs:22-33
R10 | src/tools/finding_session_tests.rs:15-26 | test | base_request duplicated verbatim in finding_race_tests.rs:22-33
R10 | src/tools/finding_tests.rs:61-73 | test | db_path-open-publish block duplicated verbatim in finding_tests.rs:90-102
R10 | src/tools/finding_tests.rs:61-73 | test | db_path-open-publish block duplicated verbatim in finding_tests.rs:124-136
R10 | src/tools/finding_tests.rs:61-73 | test | db_path-open-publish block duplicated in finding_tests.rs:144-156 (12 of 13 lines; only the final .expect message differs)
R12 | src/tools/finding.rs:234 | prod | the invalid_params message embeds the caller-supplied file path and server_runner::log_completion logs error.message at warn for every failed call (src/server_runner.rs:191), so the path reaches always-on stderr unredacted while the success log at finding.rs:173 hashes it via redacted_log_id

## Chunk 9

R1 | src/tools/leads_tests.rs:7 | test | `expect("create activation dir")` panics if the directory cannot be created.
R1 | src/tools/leads_tests.rs:36 | test | `expect("leads succeeds")` panics on any error from the helper call.
R1 | src/tools/leads_tests.rs:46 | test | `expect("leads succeeds")` panics on any error from `leads_with_limits`.
R1 | src/tools/leads_tests.rs:55 | test | `panic!` inside `unwrap_or_else` when the mechanic entry is missing.
R1 | src/tools/leads_tests.rs:67 | test | `panic!("expected full rows")` on a compact page.
R1 | src/tools/leads_tests.rs:75 | test | `panic!("expected compact rows")` on a full page.
R1 | src/tools/leads_tests.rs:91 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:97 | test | `expect("write rust source")` on `fs::write`.
R1 | src/tools/leads_tests.rs:98 | test | `expect("write .env")` on `fs::write`.
R1 | src/tools/leads_tests.rs:105 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:111 | test | `expect("write rust source")` on `fs::write`.
R1 | src/tools/leads_tests.rs:118 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:135 | test | Two `expect` calls on one line: `path.parent().expect("parent")` then `expect("create parent dirs")`.
R1 | src/tools/leads_tests.rs:136 | test | `expect("write rust source")` on `fs::write`.
R1 | src/tools/leads_tests.rs:189 | test | `pages[0]` indexes a Vec with no preceding length assertion; an empty page panics instead of failing the assert.
R1 | src/tools/leads_tests.rs:218 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:224 | test | `expect("write python source")` on `fs::write`.
R1 | src/tools/leads_tests.rs:330 | test | `full_rows(&substring)[0]` indexes a slice; only `total` is asserted, not the returned length.
R1 | src/tools/leads_tests.rs:347 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:349 | test | `expect("write .env")` on `fs::write`.
R1 | src/tools/leads_tests.rs:350 | test | `expect("create dir")` on `fs::create_dir_all`.
R1 | src/tools/leads_tests.rs:351 | test | `expect("write pem")` on `fs::write`.
R1 | src/tools/leads_tests.rs:360 | test | `full_rows(&scoped)[0]` indexes a slice; only `total` is asserted, not the returned length.
R1 | src/tools/leads_tests.rs:395 | test | `expect("add a file")` on `fs::write`.
R1 | src/tools/leads_tests.rs:398 | test | `error.data.expect(...)` panics when the refusal carries no payload.
R1 | src/tools/leads_tests.rs:400 | test | `as_str().expect("remedy is a string")` panics when the remedy is not a string.
R1 | src/tools/leads_tests.rs:423 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:429 | test | `expect("write long")` on `fs::write`.
R1 | src/tools/leads_tests.rs:431 | test | `expect("write short")` on `fs::write`.
R1 | src/tools/leads_tests.rs:469 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/leads_tests.rs:472 | test | `expect("write config")` on `fs::write`.
R1 | src/tools/leads_tests.rs:473 | test | `expect("write source")` on `fs::write`.
R1 | src/tools/leads_tests.rs:474 | test | `expect("write big.pem")` on `fs::write`.
R1 | src/tools/leads_tests.rs:491 | test | `expect("path-only request deserializes")` on a serde parse.
R1 | src/tools/leads_tests.rs:507 | test | `expect("fits")` on `i64::try_from`.
R1 | src/tools/leads_tests.rs:509 | test | `expect("rows serialize")` on `serde_json::to_value`.
R1 | src/tools/leads_tests.rs:513 | test | `expect("line")` on `as_i64`.
R1 | src/tools/leads_tests.rs:514 | test | `expect("object")` on `as_object`.
R1 | src/tools/leads_tests.rs:517 | test | `expect("rows serialize")` on `serde_json::to_value`.
R1 | src/tools/leads_tests.rs:524 | test | `compact_rows(&page)[0]` indexes a slice with no preceding length assertion on `page`.
R1 | src/tools/report_tests.rs:6 | test | `expect("create activation dir")` panics if the directory cannot be created.
R1 | src/tools/report_tests.rs:11 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:13 | test | `expect("write source")` on `fs::write`.
R1 | src/tools/report_tests.rs:14 | test | `expect("write .env")` on `fs::write`.
R1 | src/tools/report_tests.rs:15 | test | `expect("keys dir")` on `fs::create_dir_all`.
R1 | src/tools/report_tests.rs:20 | test | `expect("write pem")` on `fs::write`.
R1 | src/tools/report_tests.rs:23 | test | `expect("write ordinary source")` on `fs::write`.
R1 | src/tools/report_tests.rs:32 | test | `expect("report succeeds")` panics on any error.
R1 | src/tools/report_tests.rs:43 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:49 | test | `expect("write fixture")` on `fs::write`.
R1 | src/tools/report_tests.rs:58 | test | `expect("report succeeds")` panics on any error.
R1 | src/tools/report_tests.rs:100 | test | `expect("db dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:101 | test | `expect("open db")` on `open_read_write`.
R1 | src/tools/report_tests.rs:110 | test | `expect("insert revision")` on `execute`.
R1 | src/tools/report_tests.rs:123 | test | `expect("insert file")` on `execute`.
R1 | src/tools/report_tests.rs:129 | test | `expect("go file id")` on `query_row`.
R1 | src/tools/report_tests.rs:134 | test | `expect("python file id")` on `query_row`.
R1 | src/tools/report_tests.rs:139 | test | `expect("kotlin file id")` on `query_row`.
R1 | src/tools/report_tests.rs:152 | test | `expect("insert go edge")` on `execute`.
R1 | src/tools/report_tests.rs:159 | test | `expect("insert python edge")` on `execute`.
R1 | src/tools/report_tests.rs:166 | test | `expect("insert kotlin edge")` on `execute`.
R1 | src/tools/report_tests.rs:168 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:169 | test | `expect("build report")` on `build_report`.
R1 | src/tools/report_tests.rs:205 | test | `expect("insert one resolved and one targetless edge")` on `execute_batch`.
R1 | src/tools/report_tests.rs:207 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:208 | test | `expect("build report")` on `build_report`.
R1 | src/tools/report_tests.rs:239 | test | `expect("insert file")` on `execute`.
R1 | src/tools/report_tests.rs:242 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:243 | test | `expect("build report")` on `build_report`.
R1 | src/tools/report_tests.rs:262 | test | `expect("db dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:263 | test | `expect("open db")` on `open_read_write`.
R1 | src/tools/report_tests.rs:270 | test | `expect("insert revision and file")` on `execute_batch`.
R1 | src/tools/report_tests.rs:272 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:275 | test | `panic!` when the malformed column error is not `InvalidColumnType`.
R1 | src/tools/report_tests.rs:282 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:292 | test | `Option::expect("report on inactive project must return ErrorData")` panics if `report` succeeds.
R1 | src/tools/report_tests.rs:304 | test | `expect("db dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:306 | test | `expect("open db")` on `open_read_write`.
R1 | src/tools/report_tests.rs:314 | test | `expect("insert revision")` on `execute`.
R1 | src/tools/report_tests.rs:337 | test | `expect("insert file")` on `execute`.
R1 | src/tools/report_tests.rs:364 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:365 | test | `expect("build report")` on `build_report`.
R1 | src/tools/report_tests.rs:371 | test | `expect("indexed-but-unparsed file is listed")` on `Option::expect` from `find`.
R1 | src/tools/report_tests.rs:380 | test | `expect("genuinely failed parse is listed")` on `Option::expect` from `find`.
R1 | src/tools/report_tests.rs:399 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:400 | test | `expect("build report")` on `build_report`.
R1 | src/tools/report_tests.rs:421 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:422 | test | `expect("build report")` on `build_report`.
R1 | src/tools/report_tests.rs:437 | test | `expect("project dir")` on `tempfile::tempdir`.
R1 | src/tools/report_tests.rs:447 | test | `expect("write project config")` on `fs::write`.
R1 | src/tools/report_tests.rs:448 | test | `expect("write source")` on `fs::write`.
R1 | src/tools/report_tests.rs:449 | test | `expect("write oversize file")` on `fs::write`.
R1 | src/tools/report_tests.rs:458 | test | `expect("report succeeds")` panics on any error.
R1 | src/tools/report_tests.rs:488 | test | `expect("open the published database read-only")` on `open_read_only`.
R1 | src/tools/report_tests.rs:513 | test | `expect("the skipped file must have an existence row")` on `query_row`.
R1 | src/tools/report_tests.rs:554 | test | `expect("capped.rs id")` on `query_row`.
R1 | src/tools/report_tests.rs:563 | test | `expect("insert the evidence-cap marker")` on `execute`.
R1 | src/tools/report_tests.rs:574 | test | `expect("insert skipped row")` on `execute`.
R1 | src/tools/report_tests.rs:576 | test | `expect("begin transaction")` on `transaction`.
R1 | src/tools/report_tests.rs:577 | test | `expect("build report")` on `build_report`.
R5 | src/tools/leads.rs:331 | prod | `invalid_params("path exceeds 32768 bytes")` from the `leads` handler has no test passing an over-long path; no test asserts that message or arm.
R5 | src/tools/leads.rs:339 | prod | The `mechanic`/`pattern` 8192-byte bound in the `leads` handler is untested; no test asserts either "mechanic exceeds 8192 bytes" or "pattern exceeds 8192 bytes".
R5 | src/tools/leads.rs:349 | prod | The `expected_revision` 256-byte bound in the `leads` handler is untested; no test asserts "expected_revision exceeds 256 bytes".
R5 | src/tools/report.rs:146 | prod | `invalid_params("path exceeds 32768 bytes")` from the `report` handler is untested; no test passes an over-long path to `report`.
R5 | src/tools/report.rs:152 | prod | The `build_report` failure arm mapped to `ErrorData::internal_error` is never exercised through `report`; `malformed_language_value_is_reported_instead_of_dropped` calls `build_report` directly and never asserts this tool-level error.
R6 | src/tools/report_completeness.rs:61 | prod | The `file_kind_counts` description says `indexed` is "the set with content and structural evidence", but `indexed_files_without_evidence` (line 96) exists to count `indexed` rows with no evidence at all; the two user-visible field descriptions contradict each other.
R6 | src/tools/report_completeness.rs:88 | prod | `index_partial` documents "False means every discovered file is represented by a row here", but a discovered non-UTF-8 path gets no `files` row (`discovery.rs:89-90`, `discovery.rs:200-205`), so `skipped_file_count` stays 0 and the report returns `index_partial: false` while a discovered file is missing from the index.
R6 | src/tools/report.rs:81 | prod | `file_count` is documented as "every file the last publish discovered, whether or not it was indexed", but a discovered non-UTF-8 path is dropped from `files` entirely, so the count is smaller than the files the publish discovered.
R6 | src/tools/leads.rs:212 | prod | `PageLimit::Bytes` advises "Raise `limit` if one was set", but a byte-limited page is by construction below any `limit`; raising `limit` cannot return more rows, and the knob that acted is `page_bytes`.
R6 | src/tools/leads.rs:250 | prod | The `files_covered` description justifies cumulativeness with "rows are ordered by path", but a compact call with no mechanic orders by `ord, path, line` (`leads_compact.rs:65`), so a file's rows are split across mechanic groups rather than being contiguous under path order.
R8 | src/tools/leads.rs:187 | prod | Public enum variant `LeadRows::Full` has no doc comment.
R8 | src/tools/leads.rs:188 | prod | Public enum variant `LeadRows::Compact` has no doc comment.
R8 | src/tools/report.rs:80 | prod | Public field `ReportResponse::revision_id` has no doc comment.
R8 | src/tools/report.rs:90 | prod | Public field `ReportResponse::languages` has no doc comment.
R8 | src/tools/report.rs:91 | prod | Public field `ReportResponse::evidence_counts` has no doc comment.
R8 | src/tools/report.rs:92 | prod | Public field `ReportResponse::parser_failure_count` has no doc comment.
R8 | src/tools/report.rs:101 | prod | Public field `ReportResponse::open_finding_count` has no doc comment.
R8 | src/tools/report_completeness.rs:25 | prod | Public field `FileKindCount::count` has no doc comment.
R8 | src/tools/report_completeness.rs:33 | prod | Public field `FileClassCount::count` has no doc comment.
R8 | src/tools/report_completeness.rs:44 | prod | Public field `SkipReasonCount::count` has no doc comment.
R8 | src/tools/report_completeness.rs:55 | prod | Public field `CompletenessCount::count` has no doc comment.
R8 | src/tools/report.rs:25 | prod | Public field `LanguageCount::language` has no doc comment; the type is exposed through `ReportResponse.languages` even though rustc's missing-docs list does not reach it.
R8 | src/tools/report.rs:26 | prod | Public field `LanguageCount::file_count` has no doc comment; schema-visible through `ReportResponse.languages`.
R8 | src/tools/report.rs:32 | prod | Public field `EvidenceKindCount::kind` has no doc comment; schema-visible through `ReportResponse.evidence_counts`.
R8 | src/tools/report.rs:33 | prod | Public field `EvidenceKindCount::count` has no doc comment; schema-visible through `ReportResponse.evidence_counts`.
R8 | src/tools/report.rs:44 | prod | Public field `ImportResolutionCount::count` has no doc comment; schema-visible through `ReportResponse.import_resolution`.
R8 | src/tools/report.rs:53 | prod | Public field `ParserFailureFile::path` has no doc comment; schema-visible through `ReportResponse.parser_failure_files`.
R8 | src/tools/report.rs:54 | prod | Public field `ParserFailureFile::parse_outcome` has no doc comment; schema-visible through `ReportResponse.parser_failure_files`.
R10 | src/tools/report_tests.rs:346 | test | Lines 346-356 are duplicated verbatim at 533-543: `database_with_one_revision()` plus the identical `insert_indexed_file("notes.txt", "Unavailable", "NotAttempted", None, "ContentOnly")` setup block in two tests (11 contiguous lines).

## Chunk 10

R1 | src/tools/query_clip_tests.rs:45 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:52 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:68 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:98 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:116 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:123 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:139 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:158 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:169 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:184 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:195 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:212 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:215 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:228 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:254 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:267 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:275 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:293 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:306 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:322 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_clip_tests.rs:338 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_clip_tests.rs:349 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:34 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:49 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:51 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:66 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:79 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_page_tests.rs:93 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:119 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:130 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:131 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:149 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:158 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:167 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:190 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:207 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:230 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:246 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:257 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:274 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:294 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:319 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_page_tests.rs:325 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:40 | test | test panic! in a rejection assertion
R1 | src/tools/query_safety_tests.rs:54 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:116 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:150 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:189 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:255 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:264 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_safety_tests.rs:272 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:7 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:9 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:13 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:15 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:70 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:78 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:92 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:103 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:110 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:118 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:127 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:140 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:142 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:147 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:174 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:193 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:204 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:225 | test | test panic! inside unwrap_or_else on the query result
R1 | src/tools/query_tests.rs:229 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:242 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:246 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:251 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:255 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:262 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:267 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:275 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:297 | test | test panic! in an assertion
R1 | src/tools/query_tests.rs:306 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:308 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:315 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:327 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:342 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:355 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:364 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:367 | test | test panic! in an assertion
R1 | src/tools/query_tests.rs:377 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:379 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:399 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:420 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:443 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:445 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:448 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:461 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:462 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:478 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:480 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:487 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:489 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:490 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:492 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:504 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:529 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:539 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/query_tests.rs:559 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/query_tests.rs:579 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/redaction_tests.rs:49 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/redaction_tests.rs:79 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/redaction_tests.rs:108 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/redaction_tests.rs:127 | test | test panic! in an assertion
R1 | src/tools/redaction_tests.rs:134 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:35 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_limit_tests.rs:44 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_limit_tests.rs:63 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:113 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:217 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:232 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:301 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:302 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_limit_tests.rs:303 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:6 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:8 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:11 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:13 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:46 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:64 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_tests.rs:83 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:98 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_tests.rs:111 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_tests.rs:130 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:142 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:147 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:159 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_tests.rs:195 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:205 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:232 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_tests.rs:243 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:244 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:260 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:273 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:278 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:294 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:304 | test | test expect() call (panic is the test failure signal)
R1 | src/tools/structure_tests.rs:308 | test | test expect_err() call (panics when the call unexpectedly succeeds)
R1 | src/tools/structure_tests.rs:309 | test | test expect() call (panic is the test failure signal)
R5 | src/tools/query.rs:149 | prod | path-over-32 KiB refusal has no test asserting the exact error
R5 | src/tools/query.rs:155 | prod | expected_revision-over-256-byte refusal has no test asserting the exact error
R5 | src/tools/query_exec.rs:278 | prod | comments-only-SQL refusal (`sql must contain a statement, not only comments`) is untested
R5 | src/tools/structure.rs:158 | prod | `stale_revision` refusal from guard_expected_revision has no structure-tool test (only `query` covers it)
R5 | src/tools/structure.rs:161 | prod | unsupported-language error from tree_sitter_language_pack::get_language is untested
R5 | src/tools/structure.rs:215 | prod | structure path-over-32 KiB refusal has no test asserting the exact error
R5 | src/tools/structure.rs:217 | prod | file/language/pattern-over-8192-byte refusals have no test asserting the exact errors
R5 | src/tools/structure_source.rs:29 | prod | indexed file with NULL detected language (`has no detected language`) is untested
R5 | src/tools/structure_source.rs:157 | prod | multi-file request with neither `file` nor `language` is untested
R6 | src/tools/query.rs:25 | prod | schema text says `Only writes are rejected ... from the connection itself`, but the wrapper also refuses read-only non-SELECT statements (PRAGMA table_info, EXPLAIN SELECT 1, VACUUM) with a syntax error from the `SELECT * FROM (...)` wrapper
R6 | src/tools/structure.rs:113 | prod | `files_not_scanned` doc says 0 means every matching file after start_after_file was scanned; a page cut by bytes/match cap stops mid-window with files_not_scanned 0 while later loaded files were not scanned by this call
R7 | src/tools/structure.rs:217 | prod | validate_request bounds path/file/language/pattern/query but not `start_after_file` or `expected_revision`; both are unbounded MCP strings (query bounds expected_revision at 256 bytes)
R8 | src/tools/query.rs:75 | prod | pub field `QueryResponse::rows` has no doc comment
R8 | src/tools/query.rs:76 | prod | pub field `QueryResponse::truncated` has no doc comment
R8 | src/tools/structure.rs:95 | prod | pub field `StructureResponse::revision_id` has no doc comment
R8 | src/tools/structure.rs:96 | prod | pub field `StructureResponse::language` has no doc comment
R8 | src/tools/structure.rs:97 | prod | pub field `StructureResponse::matches` has no doc comment
R10 | src/tools/structure_tests.rs:17 | test | 20-line `ask` helper (17-36) is byte-identical to src/tools/structure_limit_tests.rs:10-29

## Chunk 11

R1 | src/bin/check_coverage.rs:211 | test | `tempfile::tempdir().expect("tempdir")` panics if the tempdir cannot be created.
R1 | src/bin/check_coverage.rs:213 | test | `remove_report(...).expect("missing file is fine")` panics on any unexpected removal error.
R1 | src/bin/check_coverage.rs:226 | test | `merge_line_totals(&cov).expect("merge")` panics if the fixture stops parsing.
R1 | src/bin/check_docs_drift/canonical_docs_tests.rs:28 | test | `remove_file(...).expect("remove fixture")` panic in fixture setup.
R1 | src/bin/check_docs_drift/canonical_docs_tests.rs:29 | test | `remove_file(...).expect("remove fixture")` panic in fixture setup.
R1 | src/bin/check_docs_drift/canonical_docs_tests.rs:51 | test | `remove_file(...).expect("remove fixture")` panic in fixture setup.
R1 | src/bin/check_docs_drift/canonical_docs_tests.rs:63 | test | `remove_file(...).expect("remove fixture")` panic in fixture setup.
R1 | src/bin/check_docs_drift/canonical_docs_tests.rs:64 | test | `create_dir(...).expect("create fixture dir")` panic in fixture setup.
R1 | src/bin/check_docs_drift/test_file_count.rs:86 | test | `tempfile::tempdir().expect("tempdir")` panic in test setup.
R1 | src/bin/check_docs_drift/test_file_count.rs:88 | test | `fs::write(...).expect("fixture")` panic in test setup.
R1 | src/bin/check_docs_drift/test_support.rs:10 | test | `fs::create_dir_all(parent).expect("create parent")` panics in the shared fixture writer.
R1 | src/bin/check_docs_drift/test_support.rs:12 | test | `fs::write(path, content).expect("write fixture")` panics in the shared fixture writer.
R1 | src/bin/check_docs_drift/test_support.rs:16 | test | `tempfile::tempdir().expect("temp dir")` panics in the shared fixture.
R1 | src/bin/check_governance/tests.rs:12 | test | `tempdir().expect("temp dir")` panic in test helper.
R1 | src/bin/check_governance/tests.rs:18 | test | `create_dir_all(...).expect("create parent directory")` panic in fixture writer.
R1 | src/bin/check_governance/tests.rs:20 | test | `fs::write(...).expect("write fixture")` panic in fixture writer.
R1 | src/bin/check_governance/tests.rs:44 | test | `remove_file(...).expect("remove fixture")` panic in fixture setup.
R1 | src/bin/check_mcp_surface/tests.rs:82 | test | `.expect("every tool declares its output schema")` panics on a missing output schema.
R1 | src/bin/check_mcp_surface/tests.rs:109 | test | `unwrap_or_else(|| panic!("tool ... is not advertised"))` panics when a documented tool is missing.
R1 | src/bin/check_mcp_surface/tests.rs:113 | test | `unwrap_or_else(|| panic!("tool ... has no annotations"))` panics on a missing annotation block.
R1 | src/bin/check_no_duplicates/commit_subjects.rs:136 | test | `tempfile::tempdir().expect("tempdir")` panic in test setup.
R1 | src/bin/check_no_duplicates/commit_subjects.rs:142 | test | `failures.first().expect("git log outside a repo must fail")` panics if no failure is recorded.
R1 | src/bin/check_no_duplicates/tests.rs:42 | test | `tempfile::tempdir().expect("temp dir creates")` panic in test setup.
R1 | src/bin/check_no_duplicates/tests.rs:44 | test | `create_dir_all(...).expect("temp src dir creates")` panic in fixture setup.
R1 | src/bin/check_no_duplicates/tests.rs:53 | test | `.expect("fixture file writes")` panic in fixture setup.
R1 | src/bin/check_no_duplicates/tests.rs:62 | test | `tempfile::tempdir().expect("temp dir creates")` panic in test setup.
R1 | src/bin/check_no_duplicates/tests.rs:64 | test | `fs::write(...).expect("fixture file writes")` panic in fixture setup.
R1 | src/bin/check_no_duplicates/tests.rs:78 | test | `tempfile::tempdir().expect("temp dir creates")` panic in test setup.
R1 | src/bin/check_no_duplicates/tests.rs:80 | test | `fs::write(...).expect("fixture file writes")` panic in fixture setup.
R1 | src/bin/check_performance/tests.rs:29 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:31 | test | `fs::write(...).expect("baseline file writes")` panic in fixture setup.
R1 | src/bin/check_performance/tests.rs:32 | test | `load_baseline(&tmp).expect("parse")` panics if the fixture stops parsing.
R1 | src/bin/check_performance/tests.rs:42 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:44 | test | `fs::write(...).expect("baseline file writes")` panic in fixture setup.
R1 | src/bin/check_performance/tests.rs:45 | test | `load_baseline(&tmp).unwrap_err()` panics if the fixture unexpectedly parses.
R1 | src/bin/check_performance/tests.rs:51 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:57 | test | `fs::write(...).expect("baseline file writes")` panic in fixture setup.
R1 | src/bin/check_performance/tests.rs:59 | test | `load_baseline(&path).expect_err(...)` panics if the malformed budget is accepted.
R1 | src/bin/check_performance/tests.rs:68 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:84 | test | `create_dir_all(...).expect("estimates dir")` panic in fixture writer.
R1 | src/bin/check_performance/tests.rs:85 | test | `fs::write(...).expect("estimates file")` panic in fixture writer.
R1 | src/bin/check_performance/tests.rs:90 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:104 | test | `collect_benches_in(...).expect("collects")` panics if the fixture stops collecting.
R1 | src/bin/check_performance/tests.rs:113 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:122 | test | `collect_benches_in(...).expect_err(...)` panics if the malformed file is accepted.
R1 | src/bin/check_performance/tests.rs:129 | test | `tempfile::tempdir().expect("temp dir")` panic in test setup.
R1 | src/bin/check_performance/tests.rs:137 | test | `collect_benches_in(...).expect_err(...)` panics if a median-less file is accepted.
R1 | src/bin/check_performance/tests.rs:263 | test | `args(...).expect("valid args parse")` panic in test.
R1 | src/bin/check_performance/tests.rs:264 | test | `args(...).expect("help parses")` panic in test.
R1 | src/bin/check_performance/tests.rs:276 | test | `bad.expect_err("non-number is a usage error")` panics if the bad arg is accepted.
R1 | src/bin/check_producer_coverage/tests.rs:111 | test | `check(root).expect("the project's own evidence enums parse")` panics if the gate errors.
R1 | src/bin/check_source_limits/tests.rs:188 | test | `exception_reason(...).expect("human annotation parses")` panic in test.
R1 | src/bin/check_source_limits/tests.rs:198 | test | `exception_reason(...).expect("human annotation parses")` panic in test.
R1 | src/bin/check_source_limits/tests.rs:208 | test | `exception_reason(...).expect("agent annotation parses")` panic in test.
R1 | src/bin/check_source_limits/tests.rs:219 | test | `exception_reason(...).expect("human annotation parses")` panic in test.
R1 | src/bin/check_version_consistency/tests.rs:16 | test | `tempdir().expect("temp dir")` panic in test helper.
R1 | src/bin/check_version_consistency/tests.rs:22 | test | `create_dir_all(...).expect("create parent directory")` panic in fixture writer.
R1 | src/bin/check_version_consistency/tests.rs:24 | test | `fs::write(...).expect("write fixture")` panic in fixture writer.
R1 | src/bin/check_version_consistency/tests.rs:35 | test | `manifests::check(root).expect("all four fixture files readable")` panic in test helper.
R1 | src/bin/check_version_consistency/tests.rs:127 | test | `remove_file(...).expect("remove fixture")` panic in fixture setup.
R1 | src/bin/check_version_consistency/tests.rs:129 | test | `manifests::check(...).expect_err(...)` panics if the missing file is not an error.
R1 | src/bin/gate_common/args.rs:74 | test | `parse_list(...).expect("no arguments is valid")` panic in test.
R1 | src/bin/gate_common/args.rs:87 | test | `.expect("valid arguments")` panic in test.
R3 | src/bin/check_coverage.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_docs_drift/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_governance/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_mcp_surface/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_no_duplicates/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_performance/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_producer_coverage/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_source_limits/main.rs:3 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R3 | src/bin/check_version_consistency/main.rs:2 | dev-gate | File-level `#![allow(clippy::print_stdout)]` lint waiver.
R5 | src/bin/check_coverage.rs:144 | dev-gate | `parse_threshold` number-parse error ("threshold ... is not a number"); the only test (`a_non_finite_or_out_of_range_threshold_is_rejected`) asserts `is_err()`, not this message.
R5 | src/bin/check_coverage.rs:147 | dev-gate | `parse_threshold` range error ("must be a finite percentage between 0 and 100"); the same test asserts `is_err()` only.
R5 | src/bin/check_docs_drift/module_map.rs:18 | dev-gate | Read-failure arm ("failed to read {doc_name}") has no test; `module_map_missing_block_is_flagged` supplies a readable ARCHITECTURE.md.
R5 | src/bin/check_docs_drift/test_file_count.rs:11 | dev-gate | Read-failure arm ("failed to read ARCHITECTURE.md") has no test.
R5 | src/bin/check_docs_drift/test_file_count.rs:21 | dev-gate | Marker-absent failure arm (sentence removed/reworded) has no test.
R5 | src/bin/check_docs_drift/test_file_count.rs:39 | dev-gate | "cannot count `*_tests.rs` files" arm has no test; the unreadable-directory test calls `count_test_files` directly.
R5 | src/bin/check_docs_drift/test_file_count.rs:52 | dev-gate | "count claim has no number before it" arm has no test.
R5 | src/bin/check_docs_drift/test_file_count.rs:61 | dev-gate | `count_test_files` read_dir error; the covering test (`an_unreadable_directory_is_an_error_not_a_zero_count`) asserts only `is_err()`.
R5 | src/bin/check_docs_drift/test_file_count.rs:65 | dev-gate | `count_test_files` per-entry error arm has no test.
R5 | src/bin/check_governance/codeowners.rs:21 | dev-gate | "CODEOWNERS is unreadable" (non-NotFound) arm has no test.
R5 | src/bin/check_no_duplicates/commit_subjects.rs:33 | dev-gate | git-not-found failure arm has no test.
R5 | src/bin/check_no_duplicates/commit_subjects.rs:37 | dev-gate | git invocation failure arm has no test.
R5 | src/bin/check_no_duplicates/commit_subjects.rs:94 | dev-gate | "git rev-parse origin/main could not run" arm has no test.
R5 | src/bin/check_performance/estimates.rs:15 | dev-gate | "criterion directory does not exist after cargo bench" arm has no test.
R5 | src/bin/check_performance/estimates.rs:41 | dev-gate | "bench directory names are not UTF-8" arm has no test.
R5 | src/bin/check_performance/estimates.rs:46 | dev-gate | Unreadable estimates-file arm has no test.
R5 | src/bin/check_performance/estimates.rs:78 | dev-gate | `walk` read_dir failure arm has no test.
R5 | src/bin/check_performance/estimates.rs:104 | dev-gate | Baseline read-failure arm has no test.
R5 | src/bin/check_performance/estimates.rs:106 | dev-gate | Baseline JSON parse-failure arm has no test (only malformed `budget_ns` and missing `benches` are covered).
R5 | src/bin/check_performance/estimates.rs:115 | dev-gate | "baseline entry ... missing median_ns" arm has no test.
R5 | src/bin/check_performance/args.rs:39 | dev-gate | "threshold ... must be a finite, non-negative percentage"; `the_cli_rejects_a_non_finite_or_negative_threshold` asserts only `is_err()`.
R5 | src/bin/check_producer_coverage/main.rs:127 | dev-gate | "no production Rust sources" error arm has no test.
R5 | src/bin/check_producer_coverage/main.rs:136 | dev-gate | Unreadable declaration-file error arm has no test.
R5 | src/bin/check_producer_coverage/main.rs:137 | dev-gate | "declares no `pub enum`" error arm has no test.
R5 | src/bin/check_producer_coverage/main.rs:144 | dev-gate | "parsed with no variants" error arm has no test.
R5 | src/bin/check_source_limits/approval.rs:25 | dev-gate | Invalid-date human approval rejection (`valid_date`) has no test; only a valid date and a wrong approver are covered.
R5 | src/bin/check_source_limits/counter.rs:103 | dev-gate | `walk_rs` read_dir failure arm has no test; no test calls `walk_rs` at all.
R5 | src/bin/check_version_consistency/manifests.rs:44 | dev-gate | "Cargo.toml has no top-level `version`" arm has no test.
R5 | src/bin/check_version_consistency/manifests.rs:54 | dev-gate | "CHANGELOG has no released `## [x.y.z]` heading" arm has no test.
R5 | src/bin/check_version_consistency/manifests.rs:72 | dev-gate | "Cargo.toml has no `tree-sitter-language-pack` pin" arm has no test.
R5 | src/bin/check_version_consistency/manifests.rs:73 | dev-gate | "parse module declares no `PACK_VERSION`" arm has no test.
R5 | src/bin/check_version_consistency/manifests.rs:109 | dev-gate | "plugin manifest has no string `version` field" arm has no test (malformed JSON is covered).
R6 | src/bin/check_docs_drift/main.rs:43 | dev-gate | Help says the gate checks "ARCHITECTURE.md's module map", but `module_map::check_module_map` prefers `.planning/MODULE_MAP.md` (module_map.rs:11-16); with the committed MODULE_MAP.md present, a stale ARCHITECTURE.md block is not checked.
R6 | src/bin/check_version_consistency/main.rs:44 | dev-gate | Help says Cargo.toml, lhm.plugin.json, CHANGELOG.md and parse::PACK_VERSION "all agree on the version"; PACK_VERSION is compared to the tree-sitter-language-pack pin (manifests.rs:59-70), not the crate version (today 1.18.0 vs 1.0.10).
R6 | src/bin/check_performance/main.rs:123 | dev-gate | PASS line rounds the threshold with `{:.0}`: with threshold `20.4`, a measured 20.3% regression passes and the message says "no benchmark regressed more than 20%".
R6 | src/bin/check_performance/format.rs:48 | dev-gate | Report header rounds the threshold with `{:.0}` (20.4 prints as 20%), misstating the gate actually applied.
R7 | src/bin/check_coverage.rs:91 | dev-gate | The coverage JSON is read whole with no size cap; the path is a fixed temp filename another local process can pre-plant.
R7 | src/bin/check_docs_drift/test_file_count.rs:68 | dev-gate | Recursive walk of the `--root` tree with no depth/entry bound and no symlink guard; a symlink cycle recurses until stack overflow.
R7 | src/bin/check_no_duplicates/commit_subjects.rs:29 | dev-gate | `Command::output()` buffers all of `git log`; with no origin/main the range is the whole history, unbounded.
R7 | src/bin/check_no_duplicates/test_names.rs:67 | dev-gate | Recursive walk (`read_dir` + `is_dir`, follows symlinks) with no depth bound.
R7 | src/bin/check_no_duplicates/test_names.rs:80 | dev-gate | Every source file is read whole with no size cap.
R7 | src/bin/check_performance/estimates.rs:82 | dev-gate | Recursive walk of `target/criterion` with no depth/symlink guard.
R7 | src/bin/check_producer_coverage/scan.rs:54 | dev-gate | Recursive walk of `src/` with no depth/symlink guard.
R7 | src/bin/check_producer_coverage/scan.rs:60 | dev-gate | Every production source file is read whole with no size cap.
R7 | src/bin/check_source_limits/counter.rs:106 | dev-gate | Recursive walk of `src/` with no depth/symlink guard.
R7 | src/bin/check_source_limits/main.rs:89 | dev-gate | Every source file is read whole with no size cap.
R10 | src/bin/check_governance/tests.rs:11-21 | test | Identical 11-line `temp_root()` + `write()` fixture helpers duplicated at src/bin/check_version_consistency/tests.rs:15-25; could live in a shared `gate_common/` module as `args.rs` does.
R12 | src/bin/check_no_duplicates/main.rs:40 | dev-gate | Each duplicate-commit failure is printed here and again from the `failures` vector at :65, so every failure is emitted twice.
R12 | src/bin/check_no_duplicates/main.rs:59 | dev-gate | Each duplicate-test-name failure is printed here and again from the `failures` vector at :65, so every failure is emitted twice.

## Chunk 12

R1 | tests/connect_tests.rs:147 | test | `.expect("write fake cli")` on `fs::write`; an unwritable fake-CLI path panics instead of reporting the I/O error.
R1 | tests/connect_tests.rs:151 | test | `.expect("metadata")` on `fs::metadata`; panics if the just-written fake CLI is not stat-able.
R1 | tests/connect_tests.rs:153 | test | `.expect("chmod fake cli")` on `set_permissions`; panics on chmod failure.
R1 | tests/connect_tests.rs:173 | test | `.expect("spawn slugaudit")` on `Command::output`; a missing/inaccessible binary panics without the spawn error reaching the test body.
R1 | tests/connect_tests.rs:178 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/connect_tests.rs:182 | test | `.expect("mkdir home")` on `create_dir_all`.
R1 | tests/connect_tests.rs:183 | test | `.expect("mkdir bin")` on `create_dir_all`.
R1 | tests/connect_tests.rs:214 | test | `.expect("truncate argv log")` on `fs::write`.
R1 | tests/connect_tests.rs:230 | test | `.expect("read argv log")` on `fs::read_to_string`; a missing log file panics.
R1 | tests/connect_tests.rs:245 | test | `panic!` inside `unwrap_or_else` when the client config written by `connect` cannot be read.
R1 | tests/connect_tests.rs:262 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/connect_tests.rs:266 | test | `.expect("mkdir cursor")` on `create_dir_all`.
R1 | tests/connect_tests.rs:267 | test | `.expect("mkdir bin")` on `create_dir_all`.
R1 | tests/connect_tests.rs:277 | test | `.expect("spawn slugaudit")` on `Command::output`.
R1 | tests/gate_binaries.rs:47 | test | `.expect("spawn gate binary")` on `Command::output`.
R1 | tests/gate_binaries.rs:52 | test | Two panicking expects on one line: `path.parent().expect("parent")` and `create_dir_all(...).expect("mkdir")`.
R1 | tests/gate_binaries.rs:53 | test | `.expect("write fixture")` on `fs::write`.
R1 | tests/gate_binaries.rs:58 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/gate_binaries.rs:74 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/gate_binaries.rs:75 | test | `.expect("utf-8 temp path")` on `Path::to_str`.
R1 | tests/gate_binaries.rs:90 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/gate_binaries.rs:91 | test | `.expect("utf-8 temp path")` on `Path::to_str`.
R1 | tests/large_repo_soak.rs:92 | test | `.expect("create project temp dir")` on `tempfile::tempdir`.
R1 | tests/large_repo_soak.rs:94 | test | `.expect("create activation dir")` on `create_dir_all`.
R1 | tests/large_repo_soak.rs:110 | test | `.expect("create synthetic module directory")` on `create_dir_all`.
R1 | tests/large_repo_soak.rs:134 | test | `.expect("write synthetic source file")` on `fs::write`.
R1 | tests/large_repo_soak.rs:146 | test | `.expect("count rows")` on `Connection::query_row`.
R1 | tests/large_repo_soak.rs:147 | test | `.expect("row count fits in usize")` on `usize::try_from`; a negative count panics.
R1 | tests/large_repo_soak.rs:154 | test | `.expect("prepare")` on `Connection::prepare`.
R1 | tests/large_repo_soak.rs:157 | test | `.expect("query")` on `Statement::query_map`.
R1 | tests/large_repo_soak.rs:159 | test | `.expect("collect stored paths")` on the collect of row Results; a malformed row panics.
R1 | tests/large_repo_soak.rs:184 | test | `panic!` when `ensure_current` returns Err instead of asserting on the typed sync failure.
R1 | tests/large_repo_soak.rs:194 | test | `.expect("open the published database")` on `open_read_only`.
R1 | tests/menu_tests.rs:21 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/menu_tests.rs:24 | test | `.expect("mkdir home")` on `create_dir_all`.
R1 | tests/menu_tests.rs:25 | test | `.expect("mkdir empty path")` on `create_dir_all`.
R1 | tests/menu_tests.rs:38 | test | `.expect("spawn slugaudit menu")` on `Command::spawn`.
R1 | tests/menu_tests.rs:41 | test | `.expect("menu stdin")` on `Child::stdin.take()`.
R1 | tests/menu_tests.rs:44 | test | `.expect("write menu choice")` on `writeln!` into the child's stdin.
R1 | tests/menu_tests.rs:50 | test | `.expect("menu stdout")` on `Child::stdout.take()`.
R1 | tests/menu_tests.rs:54 | test | `.expect("menu stdout receiver is alive")` on the capture thread's channel send.
R1 | tests/menu_tests.rs:56 | test | `.expect("menu stderr")` on `Child::stderr.take()`.
R1 | tests/menu_tests.rs:60 | test | `.expect("menu stderr receiver is alive")` on the capture thread's channel send.
R1 | tests/menu_tests.rs:65 | test | `.expect("try_wait menu")` on `Child::try_wait`.
R1 | tests/menu_tests.rs:71 | test | `panic!` when the menu does not exit within `MENU_TIMEOUT`.
R1 | tests/menu_tests.rs:78 | test | `.expect("menu stdout capture must finish")` on `recv_timeout`.
R1 | tests/menu_tests.rs:79 | test | `.expect("menu stdout must be readable")` on the captured `read_to_string` Result.
R1 | tests/menu_tests.rs:82 | test | `.expect("menu stderr capture must finish")` on `recv_timeout`.
R1 | tests/menu_tests.rs:83 | test | `.expect("menu stderr must be readable")` on the captured `read_to_string` Result.
R1 | tests/menu_tests.rs:138 | test | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | tests/menu_tests.rs:139 | test | `.expect("temp path is utf-8")` on `Path::to_str`.
R1 | tests/stdio_protocol.rs:28 | test | `.expect("spawn slugaudit")` on `Command::spawn`.
R1 | tests/stdio_protocol.rs:30 | test | `.expect("stdin")` on `Child::stdin.take()`.
R1 | tests/stdio_protocol.rs:31 | test | `.expect("stdout")` on `Child::stdout.take()`.
R1 | tests/stdio_protocol.rs:32 | test | `.expect("stderr")` on `Child::stderr.take()`.
R1 | tests/stdio_protocol.rs:74 | test | `.expect("stdin still open")` on `Option::as_mut`; sending after close panics.
R1 | tests/stdio_protocol.rs:75 | test | `.expect("write request line")` on `writeln!` to the server.
R1 | tests/stdio_protocol.rs:76 | test | `.expect("flush request")` on `Write::flush`.
R1 | tests/stdio_protocol.rs:82 | test | `.expect("a stdout line within timeout")` on `recv_timeout`; a silent server panics the harness.
R1 | tests/stdio_protocol.rs:96 | test | `.expect("stdout line is valid JSON")` on `serde_json::from_str`.
R1 | tests/stdio_protocol.rs:121 | test | `panic!` inside `unwrap_or_else` when a tool response line is not JSON.
R1 | tests/stdio_protocol.rs:136 | test | `panic!` inside `unwrap_or_else` when a tool result has no text content.
R1 | tests/stdio_protocol.rs:138 | test | `panic!` inside `unwrap_or_else` when the result text is not JSON.
R1 | tests/stdio_protocol.rs:156 | test | `.expect("project dir")` on `tempfile::tempdir`.
R1 | tests/stdio_protocol.rs:158 | test | `.expect("activate project")` on `create_dir_all`.
R1 | tests/stdio_protocol.rs:160 | test | `.expect("write fixture file")` on `fs::write`.
R1 | tests/stdio_protocol.rs:170 | test | `.expect("init response")` on parsing the initialize response.
R1 | tests/stdio_protocol.rs:209 | test | `.expect("languages array")` on `Value::as_array`; a shape change panics.
R1 | tests/stdio_protocol.rs:267 | test | `.expect("modify source")` on `fs::write`.
R1 | tests/stdio_protocol.rs:282 | test | `.expect("revision count is a number")` on `Value::as_i64`.
R1 | tests/stdio_protocol.rs:346 | test | `.expect("report response")` on parsing the report response.
R1 | tests/stdio_protocol.rs:349 | test | `.expect("report text")` on `Value::as_str`.
R1 | tests/stdio_protocol.rs:350 | test | `.expect("report JSON")` on parsing the report text.
R1 | tests/stdio_protocol.rs:353 | test | `.expect("revision id")` on `Value::as_str`.
R1 | tests/stdio_protocol.rs:438 | test | `.expect("at least one progress notification")` on `Vec::last`; panics if the token-carrying call produced no notification at all.
R1 | benches/common/mod.rs:62 | bench | `sibling` computes `(k * 7 + 3) % count` with no zero guard; `generate_fixture(root, 2..=9)` yields a zero per-language count and panics on modulo by zero (e.g. file_count=2 -> rust_count=0).
R1 | benches/common/mod.rs:75 | bench | `.expect("activation dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:76 | bench | `.expect("rust dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:77 | bench | `.expect("python dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:78 | bench | `.expect("js dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:79 | bench | `.expect("ts dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:80 | bench | `.expect("docs dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:81 | bench | `.expect("config dir")` on `create_dir_all`.
R1 | benches/common/mod.rs:95 | bench | `.expect("write fixture file")` on `fs::write`.
R1 | benches/discovery.rs:22 | bench | `.expect("discover")` on `discover`; a walk error panics the benchmark.
R1 | benches/discovery.rs:27 | bench | `.expect("discover")` on `discover` in the walk+hash pipeline.
R1 | benches/discovery.rs:32 | bench | `.expect("hash")` on `hash_file`; a per-file hash error aborts the measurement.
R1 | benches/discovery.rs:40 | bench | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | benches/discovery.rs:41 | bench | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | benches/memory.rs:56 | bench | `.expect("project dir")` on `tempfile::tempdir`.
R1 | benches/memory.rs:57 | bench | `.expect("activation dir")` on `create_dir_all`.
R1 | benches/memory.rs:70 | bench | `.expect("db dir")` on `tempfile::tempdir` inside the timed loop.
R1 | benches/memory.rs:73 | bench | `.expect("open db")` on `open_read_write`.
R1 | benches/memory.rs:77 | bench | `.expect("resolve project root")` on `ProjectRoot::resolve`.
R1 | benches/memory.rs:78 | bench | `.expect("activate project")` on `project::enable`.
R1 | benches/memory.rs:86 | bench | `.expect("sync")` on `ensure_current`; any sync error panics mid-benchmark.
R1 | benches/memory.rs:106 | bench | `panic!` when peak RSS cannot be measured, aborting after the criterion loop already ran.
R1 | benches/parsing.rs:34 | bench | `.expect("extract")` on `evidence::extract` inside the timed closure.
R1 | benches/parsing.rs:67 | bench | `panic!` when a grammar used by the extract benches fails to load.
R1 | benches/search.rs:35 | bench | `.expect("temp dir")` on `tempfile::tempdir`.
R1 | benches/search.rs:37 | bench | `.expect("utf-8 fixture path")` on `Path::to_str`.
R1 | benches/search.rs:40 | bench | `.expect("initial sync")` on `ensure_current`.
R1 | benches/search.rs:41 | bench | `.expect("valid project root")` on `ProjectRoot::resolve`.
R1 | benches/search.rs:50 | bench | `.expect("prepare search SQL")` on `Connection::prepare`.
R1 | benches/search.rs:51 | bench | `.expect("execute search SQL")` on `Statement::query`.
R1 | benches/search.rs:52 | bench | Two panicking expects on one line: `rows.next().expect("one result row").expect("read row")`.
R1 | benches/search.rs:53 | bench | `.expect("count column")` on `Row::get`.
R1 | benches/search.rs:73 | bench | `.expect("open read-only")` on `open_read_only`.
R1 | benches/search.rs:74 | bench | `.expect("open read-only")` on `open_read_only`.
R1 | benches/startup.rs:35 | bench | `.expect("spawn slugaudit-mcp")` on `Command::spawn`.
R1 | benches/startup.rs:37 | bench | `.expect("stdin pipe")` on `Child::stdin.take()`.
R1 | benches/startup.rs:38 | bench | `.expect("stdout pipe")` on `Child::stdout.take()`.
R1 | benches/startup.rs:43 | bench | `.expect("write initialize")` on `write_all`.
R1 | benches/startup.rs:49 | bench | `.expect("read initialize response")` on `read_to_end`.
R1 | fuzz/fuzz_targets/support.rs:179 | fuzz | `.expect("create fixture temp dir")` on `tempdir`; a fuzz process aborts on fixture setup failure.
R1 | fuzz/fuzz_targets/support.rs:186 | fuzz | `.expect("create activation dir")` on `create_dir_all`.
R1 | fuzz/fuzz_targets/support.rs:189 | fuzz | `.expect("fixture file has a parent")` on `Path::parent`; a bare relative fixture name would panic.
R1 | fuzz/fuzz_targets/support.rs:190 | fuzz | `.expect("create fixture dir")` on `create_dir_all`.
R1 | fuzz/fuzz_targets/support.rs:191 | fuzz | `.expect("write fixture file")` on `fs::write`.
R1 | fuzz/fuzz_targets/support.rs:201 | fuzz | `.expect("fixture sync must succeed...")` on `ensure_synced`; aborts the fuzz process rather than reporting.
R2 | tests/menu_tests.rs:69 | test | `let _ = child.kill();` discards the kill error; if the kill fails the test panics with only "did not exit within", losing the cause.
R2 | tests/menu_tests.rs:70 | test | `let _ = child.wait();` discards the reap error on the timeout path.
R2 | tests/stdio_protocol.rs:187 | test | `let _ = self.child.kill();` in `Drop` discards the kill error; a failed kill leaks the server process silently.
R2 | tests/stdio_protocol.rs:188 | test | `let _ = self.child.wait();` in `Drop` discards the reap/exit-status error.
R2 | benches/startup.rs:52 | bench | `let _ = child.wait();` discards the server's `ExitStatus`; a server that answers initialize then exits non-zero is benchmarked and reported as a success.
R2 | fuzz/fuzz_targets/request_arguments.rs:70 | fuzz | `let _ = serde_json::from_slice::<$request>(body);` discards every decode error; a contract regression that rejects all inputs is indistinguishable from expected refusals and the target stays green.
R2 | fuzz/fuzz_targets/query_sql.rs:64 | fuzz | `if let Ok(Json(response)) = query(...)` without an else silently discards every error; a regression that makes `query` refuse all SQL (including unexpected internal errors) leaves the target green.
R2 | fuzz/fuzz_targets/structure_query.rs:94 | fuzz | `if let Ok(Json(response)) = structure(...)` without an else silently discards every error; a regression that makes `structure` refuse all queries leaves the target green.
R6 | benches/common/mod.rs:33 | bench | `fixture_stats` prints `files=self.file_count` (`generate_fixture` at line 165 stores the requested loop count), but the generated tree also writes `.gitignore`, `src/python/broken.py` and `src/js/broken.js`; the log says `files=40` for a 43-file fixture, while `bytes` does include those files, so the two counters describe different trees.
R10 | tests/stdio_protocol.rs:34-48,49-63 | test | Duplicated 15-line stdout line-reader thread block (34-48), identical to the stderr block (49-63) except channel names and the message; a shared `spawn_line_reader(reader, label)` helper could own both.

## Detector-derived (R4/R9/R11)

R4 | src/tools/report_tests.rs:1 | test | file is 498 code lines (>200)
R4 | src/store/connection_tests.rs:1 | test | file is 493 code lines (>200)
R4 | src/tools/leads_tests.rs:1 | test | file is 482 code lines (>200)
R4 | src/connect_registry_tests.rs:1 | test | file is 477 code lines (>200)
R4 | src/tools/query_tests.rs:1 | test | file is 477 code lines (>200)
R4 | src/sync/reconcile/reconcile_tests.rs:1 | test | file is 445 code lines (>200)
R4 | src/connect_tests.rs:1 | test | file is 432 code lines (>200)
R4 | src/project/activation_tests.rs:1 | test | file is 410 code lines (>200)
R4 | src/sync/manager_tests.rs:1 | test | file is 402 code lines (>200)
R4 | src/sync/sample_tests.rs:1 | test | file is 402 code lines (>200)
R4 | src/install_tests.rs:1 | test | file is 394 code lines (>200)
R4 | src/tools/finding_tests.rs:1 | test | file is 390 code lines (>200)
R4 | src/sync/sweep_tests.rs:1 | test | file is 370 code lines (>200)
R4 | src/evidence/normalize_tests.rs:1 | test | file is 350 code lines (>200)
R4 | src/sync/publish_mutation_tests.rs:1 | test | file is 333 code lines (>200)
R4 | src/watch/manager_event_tests.rs:1 | test | file is 322 code lines (>200)
R4 | src/tools/project_control_tests.rs:1 | test | file is 316 code lines (>200)
R4 | src/sync/manager_observability_tests.rs:1 | test | file is 308 code lines (>200)
R4 | src/tools/query_clip_tests.rs:1 | test | file is 308 code lines (>200)
R4 | src/tools/structure_limit_tests.rs:1 | test | file is 298 code lines (>200)
R4 | src/watch/manager.rs:1 | prod | file is 295 code lines (>200)
R4 | src/server_runner_tests.rs:1 | test | file is 279 code lines (>200)
R4 | src/sync/publish_attempt.rs:1 | prod | file is 273 code lines (>200)
R4 | src/sync/publish_edges_tests.rs:1 | test | file is 272 code lines (>200)
R4 | src/tools/structure_tests.rs:1 | test | file is 272 code lines (>200)
R4 | src/tools/query_page_tests.rs:1 | test | file is 270 code lines (>200)
R4 | src/connect_agents.rs:1 | prod | file is 261 code lines (>200)
R4 | src/tools/health_state_tests.rs:1 | test | file is 257 code lines (>200)
R4 | src/evidence/normalize.rs:1 | prod | file is 249 code lines (>200)
R4 | src/connect_exec.rs:1 | prod | file is 246 code lines (>200)
R4 | src/sync/hash.rs:1 | prod | file is 243 code lines (>200)
R4 | src/sync/sample.rs:1 | prod | file is 242 code lines (>200)
R4 | src/tools/finding_session_tests.rs:1 | test | file is 241 code lines (>200)
R4 | src/sync/publish_tests.rs:1 | test | file is 239 code lines (>200)
R4 | src/sync/reconcile/pipeline.rs:1 | prod | file is 238 code lines (>200)
R4 | src/util_tests.rs:1 | test | file is 235 code lines (>200)
R4 | src/store/connection.rs:1 | prod | file is 234 code lines (>200)
R4 | src/util.rs:1 | prod | file is 233 code lines (>200)
R4 | src/server.rs:1 | prod | file is 232 code lines (>200)
R4 | src/update_tests.rs:1 | test | file is 231 code lines (>200)
R4 | src/bin/check_performance/tests.rs:1 | test | file is 230 code lines (>200)
R4 | src/sync/manager_health.rs:1 | prod | file is 229 code lines (>200)
R4 | src/sync/publish_race_tests.rs:1 | test | file is 227 code lines (>200)
R4 | src/ignore_rules_tests.rs:1 | test | file is 224 code lines (>200)
R4 | src/store/netfs_tests.rs:1 | test | file is 219 code lines (>200)
R4 | src/tools/finding.rs:1 | prod | file is 218 code lines (>200)
R4 | src/sync/timeout_tests.rs:1 | test | file is 213 code lines (>200)
R4 | src/tools/health.rs:1 | prod | file is 213 code lines (>200)
R4 | src/graph/resolve_rust.rs:1 | prod | file is 211 code lines (>200)
R4 | src/tools/query_exec.rs:1 | prod | file is 210 code lines (>200)
R4 | src/sync/manager_meta_tests.rs:1 | test | file is 208 code lines (>200)
R4 | src/tools/report.rs:1 | prod | file is 205 code lines (>200)
R4 | src/project/activation.rs:1 | prod | file is 202 code lines (>200)
R4 | src/tools/query_safety_tests.rs:1 | test | file is 201 code lines (>200)
R4 | benches/common/mod.rs:71 | bench | function is 86 lines (>50)
R4 | src/bin/check_coverage.rs:35 | dev-gate | function is 87 lines (>50)
R4 | src/bin/check_docs_drift/module_map.rs:10 | dev-gate | function is 75 lines (>50)
R4 | src/bin/check_mcp_surface/diff.rs:22 | dev-gate | function is 55 lines (>50)
R4 | src/bin/check_source_limits/main.rs:49 | dev-gate | function is 69 lines (>50)
R4 | src/connect_registry_tests.rs:250 | test | function is 72 lines (>50)
R4 | src/connect_registry_tests.rs:59 | test | function is 51 lines (>50)
R4 | src/model/profile.rs:94 | prod | function is 70 lines (>50)
R4 | src/server_runner_tests.rs:207 | test | function is 57 lines (>50)
R4 | src/sync/discovery.rs:146 | prod | function is 62 lines (>50)
R4 | src/sync/manager_meta_tests.rs:106 | test | function is 53 lines (>50)
R4 | src/sync/publish_acceptance_tests.rs:70 | test | function is 96 lines (>50)
R4 | src/sync/publish_diff.rs:35 | prod | function is 55 lines (>50)
R4 | src/sync/publish_mutation_tests.rs:253 | test | function is 54 lines (>50)
R4 | src/sync/publish_mutation_tests.rs:321 | test | function is 83 lines (>50)
R4 | src/sync/publish_race_tests.rs:149 | test | function is 60 lines (>50)
R4 | src/sync/publish_race_tests.rs:224 | test | function is 51 lines (>50)
R4 | src/sync/reconcile/pipeline.rs:100 | prod | function is 56 lines (>50)
R4 | src/sync/reconcile/pipeline.rs:199 | prod | function is 77 lines (>50)
R4 | src/sync/reconcile/reconcile_binary_tests.rs:16 | test | function is 54 lines (>50)
R4 | src/sync/reconcile/reconcile_ignore_tests.rs:15 | test | function is 55 lines (>50)
R4 | src/sync/revision_files.rs:27 | prod | function is 57 lines (>50)
R4 | src/sync/sample.rs:223 | prod | function is 51 lines (>50)
R4 | src/sync/sweep_tests.rs:363 | test | function is 59 lines (>50)
R4 | src/tools/context_race_tests.rs:34 | test | function is 84 lines (>50)
R4 | src/tools/finding_race_tests.rs:56 | test | function is 58 lines (>50)
R4 | src/tools/finding.rs:95 | prod | function is 70 lines (>50)
R4 | src/tools/finding_session_tests.rs:104 | test | function is 70 lines (>50)
R4 | src/tools/finding_session_tests.rs:255 | test | function is 62 lines (>50)
R4 | src/tools/leads.rs:314 | prod | function is 61 lines (>50)
R4 | src/tools/mod.rs:254 | prod | function is 58 lines (>50)
R4 | src/tools/query_clip.rs:72 | prod | function is 68 lines (>50)
R4 | src/tools/query_exec.rs:165 | prod | function is 66 lines (>50)
R4 | src/tools/query.rs:135 | prod | function is 77 lines (>50)
R4 | src/tools/report_completeness.rs:110 | prod | function is 51 lines (>50)
R4 | src/tools/report_tests.rs:436 | test | function is 83 lines (>50)
R4 | src/tools/report_tests.rs:532 | test | function is 86 lines (>50)
R4 | src/tools/report_tests.rs:97 | test | function is 76 lines (>50)
R4 | src/tools/structure_exec.rs:124 | prod | function is 61 lines (>50)
R4 | src/tools/structure.rs:146 | prod | function is 53 lines (>50)
R4 | src/watch/scope.rs:89 | prod | function is 78 lines (>50)
R4 | tests/connect_tests.rs:177 | test | function is 71 lines (>50)
R4 | tests/large_repo_soak.rs:166 | test | function is 59 lines (>50)
R4 | tests/menu_tests.rs:12 | test | function is 60 lines (>50)
R4 | tests/stdio_protocol.rs:198 | test | function is 105 lines (>50)
R4 | src/bin/check_coverage.rs:35 | dev-gate | function main has cyclomatic complexity 16 (>15)
R4 | src/bin/check_source_limits/main.rs:49 | dev-gate | function main has cyclomatic complexity 16 (>15)
R4 | src/watch/scope.rs:89 | prod | function WatchScope::compute_with_limits has cyclomatic complexity 18 (>15)
R4 | src/bin/check_mcp_surface/diff.rs:22 | dev-gate | function walk has cyclomatic complexity 19 (>15)
R4 | src/sync/reconcile/pipeline.rs:199 | prod | function reconcile_dirty_paths_with_deadline has cyclomatic complexity 18 (>15)
R4 | src/bin/check_version_consistency/manifests.rs:24 | dev-gate | function check has cyclomatic complexity 18 (>15)
R4 | src/model/limits.rs:156 | prod | function ResourceLimits::from_env has cyclomatic complexity 17 (>15)
R4 | src/sync/reconcile/pipeline.rs:100 | prod | function reconcile_single_dirty_path has cyclomatic complexity 17 (>15)
R4 | src/bin/check_producer_coverage/scan.rs:149 | dev-gate | function enum_variants has cyclomatic complexity 16 (>15)
R4 | src/tools/query_exec.rs:165 | prod | function execute_and_collect has cyclomatic complexity 16 (>15)


## Appendix C — verification-pass suspected misses (not counted)

### Chunk 1
R6 | src/resources.rs:51 | The `auth-attack-surface` description says it maps "functions, methods, and calls touching auth, tokens, or sessions", but the embedded recipe (`examples/queries/auth_attack_surface.sql:10-16`) LIKEs the whole JSON payload for auth/token/session/secret/password, so caller names and symbol names merely containing `auth` (e.g. `author`, `handle_unauthenticated`) match calls and symbols that do not touch auth. Same over-match class as the confirmed `find-unhandled-panics` entry at `src/resources.rs:44`.
R6 | src/server.rs:274-277 | The `query` tool description enumerates the evidence kinds as "Symbol, Import, Comment, Structure", omitting Export, Docstring, Diagnostic, and Call, which the index produces and `INSTRUCTIONS` lists (`src/server.rs:73-74`). An agent reading the tool description can conclude Call/Diagnostic evidence is not queryable; low confidence because the text may be intended as examples rather than an exhaustive list.

### Chunk 2
R5 | src/connect_exec.rs:238 | `ConnectError::Io` from a non-`NotFound` `read_file_capped` failure in `load_json_or_empty` has no test asserting the variant (the invalid-JSON test covers `InvalidConfig` only); a distinct arm from the cited Io sites.
R5 | src/connect_exec.rs:294 | `path.try_exists()?` in `remove_json_slugaudit` converts to `ConnectError::Io`; the only covering test (connect_registry_tests.rs:507) asserts `is_err()`, not the variant — the same standard applied to connect.rs:59.
R5 | src/menu.rs:119 | `confirm_serve`'s `stdout().flush()?` and the stdin read at 122 are the same untested `Box<dyn Error>` failure paths as the cited run_menu lines, but are not reported.
R5 | src/connect_exec.rs:266 | `ConnectError::Json` from `serde_json::to_string_pretty(...)?` (also 301) is a public error variant with no test; reachability is near-zero because serializing a `Value` is infallible in practice.
R5 | src/update_fetch.rs:106 | `UpdateError::Download`, the wrapper every download failure returns, is only constructed in a Display/source test (update_tests.rs:208); its inner `ToolSpawn`/`Network` are reported but the wrapper variant has no failure-path assertion.
R12 | src/ignore_rules/build.rs:64 | The "ignore file has no parent" warning logs `file.display()` raw, while DECISIONS 2026-10-16 (c) and the module's own comment (lines 25-28) say ignore-matcher diagnostics use project-relative paths; sibling sites use `rel(root, ..)`.
R7 | src/ignore_rules/build.rs:143 | Ignore-file contents reach `GitignoreBuilder::add` unbounded: ignore 0.4.33 reads each file with `read_to_end` and no size cap (registry `gitignore.rs:613`), unlike the project's other foreign-text reads capped at `MAX_FOREIGN_TEXT_BYTES`; a huge `.gitignore` is read whole during discovery.

### Chunk 3
R8 | src/model/span.rs:189 | Public variant `SpanError::ReversedPosition` has a doc comment but no reference outside `model::span`: its only external occurrence is `src/model/span_tests.rs:26`, a `#[cfg(test)]` sibling of the same module. Same standard the author applied to `char_column`/`byte_len`, whose only uses are also their own tests.

### Chunk 4
R10 | src/project/activation.rs:58-69 | Exact 10-line duplication with activation.rs:74-87: the `Err(error)` guard, `matches!`, `error.kind()`, NotFound|NotADirectory, `) =>`, `{`, `Ok(false)`, `}` and both trailing arms are line-for-line identical; only the `Ok(metadata)` line differs (`is_symlink` vs `is_dir`). The ledger's R10 entry cites this pair only parenthetically; its primary connection.rs pairing is dropped above.
R10 | src/store/connection_tests.rs:72-81 | Exact 10-line `for (label, connection) in [ ... ]` fixture setup duplicated at src/store/connection_tests.rs:623-632 (ATTACH test vs line-functions test); no ledger entry.
R10 | src/store/migrations_tests.rs:82-93 | 12-line `assert!(matches!(err, MigrationError::UnsupportedVersion { .. }))` + `assert!(err.is_corruption(), ..)` block duplicated at migrations_tests.rs:104-115; only `found: 1` vs `found: 2` differs. No ledger entry.
R10 | src/project/root.rs:48-54 | Seven-line canonicalize assertion (`assert_eq!(root.as_path(), directory.path().canonicalize().expect(..))`) duplicated verbatim at activation_tests.rs:72-78 and 91-97. Below the 10-line bar unless surrounding blank/brace/attribute lines are counted, so low confidence.
R5 | src/store/connection.rs:228 | `StoreError::Permissions` from `create_with_private_permissions_if_missing` (a `create_new` failure other than AlreadyExists) has no test; the two Permissions assertions (connection_tests.rs:226 and :334) drive only the existing-file mode rejection at connection.rs:263.
R5 | src/store/connection.rs:246 | `StoreError::Permissions` from the `std::fs::metadata` failure arm of `reject_insecure_permissions` has no test; no test forces a stat failure other than NotFound at this site.
R5 | src/store/connection.rs:298 | `open_read_write`'s `sql_functions::register` Configure arm (and `configure()`'s busy_timeout arm at connection.rs:347) are untested; connection_tests.rs:127 reaches only the journal_mode arm at :336. The ledger reports the equivalent open_read_only arms but not these.
R5 | src/store/netfs.rs:87 | `StoreError::NetworkFilesystemCheck` from `read_to_string("/proc/self/mountinfo")` has no test; the tested cases are the non-UTF-8 path (netfs_tests.rs:273) and the unsupported target (:199). The macOS `/sbin/mount` failure arms (netfs.rs:203-208) are also untested on Linux CI.
R5 | src/project/config.rs:69 | `ConfigError::Io` from `read_file_capped` has no test; the only Io assertion (config.rs:221) drives the earlier `try_exists` arm at config.rs:60.
R6 | src/store/connection.rs:34 | `StoreError::Permissions`' message "failed to set database file permissions" is also returned when file creation fails (:228) or metadata inspection fails (:246), and for the refusal at :263, where no permission was set; the message misnames the operation.
R6 | src/project/activation.rs:28 | `ActivationError::Io` says "failed to update the activation directory", but the variant also carries inspection failures from `find_project_root`/`validated_activation_dir` (`is_symlink`, `is_existing_dir`) and `try_exists` in `disable`, where nothing was updated.
R7 | src/store/netfs.rs:86 | `read_to_string("/proc/self/mountinfo")` (and the macOS `Command::output` at :203) reads the whole mount table with no length cap. System-provided rather than agent input, so lower severity.
R7 | src/watch/state.rs:39 | `dirty_paths`/`deleted_paths` have no per-project cap between reconciles; `MAX_DEFERRED_EVENTS` (manager.rs:53) bounds only the deferred queue, so a churning tree can retain an unbounded set of path strings in each project's state.
R8 | src/watch/scope.rs:77 | `WatchScope::compute` is public and documented but has no non-test caller: production uses `compute_with_limits` (manager.rs:230,366), and the only callers are ignore_rules_tests.rs:20, reconcile_ignore_tests.rs:21 and scope_tests.rs.

### Chunk 5
R5 | src/sync/manager_meta.rs:247 | Manager-path publish failures are integration-tested only for `is_err()` (manager_observability_tests.rs:140-144 and 164-168); `publish_failures_classify_into_the_documented_error_codes` constructs `PublishError`s directly and asserts only `data`, so no test asserts the message/`data` of the ErrorData returned from a real failing publish.
R12 | src/sync/analyze.rs:181 | A genuine parse/extraction failure (`parse_failed_result`) is logged at `debug`, below R12's warn/error bar; this is the open R12-4 item from the 2026-10-16 audit (the outcome also reaches the caller via `ParseOutcome::Failed`, so the lead should judge whether that satisfies "reaches the caller").
R5 | src/sync/manager_health.rs:337 | `sweep_error_to_sync`'s `SweepError::Database` -> `SyncError::Sweep` mapping is untested; manager_tests.rs:480 constructs `SyncError::Sweep` directly instead of driving the mapping.

### Chunk 6
R10 | src/sync/publish_edges_tests.rs:45 | The 10-line `db_dir`/`open_read_write`/`publish` block at 45-54 is byte-identical at 81-90, 108-117 and 190-199, and at src/sync/publish_tests.rs:18-27 and 196-205; no ledger R10 entry covers this pair/cluster.
R10 | src/sync/publish_edges_tests.rs:135 | The 10-line block at 135-144 (db open + first publish) is byte-identical at src/sync/publish_tests.rs:154-163 and 239-248; not in the ledger.
R10 | src/sync/publish_tests.rs:87 | The 10-line first-publish setup at 87-96 is byte-identical at 115-124; not in the ledger.
R7 | src/sync/revision.rs:270 | `invalidate_stale_findings` calls `paths_json_array` on the full upsert+deletion+skipped set on every publish_revision, i.e. the full-publish path where a first import touches every file; same unbounded JSON-allocation/SQL-value exposure as the reported manifest.rs:113 site (revision.rs is outside chunk 6).
R7 | src/sync/publish_diff.rs:48 | `diff_against_stored` materializes every stored `files` row (and every sample) into in-memory Vecs with no row cap or deadline check, the same unbounded-file-count input.
R12 | src/sync/manager_meta.rs:233 | The second warn line of the same publish failure also logs `error = %error`, repeating the raw absolute-path leak of publish_log.rs:16 (the ledger cites this line as :232; manager_meta.rs is outside chunk 6).
R5 | src/sync/discovery.rs:43 | Publicly re-exported `DiscoveryError` variants `Walk` (43), `NotRelative` (51) and `NonUtf8Path` (56) have no test asserting the exact variant; discovery_tests.rs:118 only checks the skipped count/reason (discovery.rs is outside chunk 6).
R6 | src/sync/publish_cas.rs:94 | When the wall-clock deadline is spent and the final attempt failed with a retryable error, the loop returns that error (e.g. "concurrent publish won the race; retry the call") instead of `TimeBudgetExceeded` promised by the doc at 61-64, and the caller gets no `sync_budget_exhausted` code — low confidence, doc-vs-behavior.
R7 | src/sync/publish_cas.rs:72 | `parser_pack_version: &str` on public `publish` has no length cap; validation scans it and `InvalidParserPackVersion(String)` clones it. Low impact: all in-crate callers pass `parse::PACK_VERSION`.

### Chunk 7
R10 | src/sync/reconcile/reconcile_tests.rs:38 | Additional qualifying pair the ledger does not list: `modified_dirty_paths_are_reindexed` lines 38-50 and `missing_dirty_file_is_treated_as_deleted` lines 156-168 share a 13-line contiguous block; several other 10-12 line pairs exist among the first eight tests (e.g. 15-26/182-193, 99-110/159-170). Likely one shared-scaffolding issue rather than distinct findings.
R7 | src/sync/revision.rs:270 | `paths_json_array(touched_paths)` serializes every upsert/deletion/skipped path into one JSON parameter with no count or byte cap, after the read-side lookup was deliberately batched (queries.rs:9-17). Discovery is bounded only by `max_sync_wall_clock`, and `max_total_import_bytes` does not count zero-byte files, so a very large tree of empty/small files builds an unbounded string inside the publish transaction.

### Chunk 8
R5 | src/tools/finding.rs:192-195 | per-field validation arms have no test asserting their specific message: title over 200 chars, description empty, severity empty/over 100 chars, category empty/over 100 chars. Only title-empty (finding_tests.rs:165) and description-oversize (finding_tests.rs:246) exercise check_field's two branches, so the distinct per-field errors are untested.
R6 | src/tools/finding.rs:171 | "finding recorded" is logged inside insert_finding, before with_verified_write commits (context_transactions.rs:33); if the commit fails, the caller gets an error after the success line was already emitted, so the log claims a record that rolled back.
R6 | src/tools/health.rs:294 | a server-side resolution failure (unreadable parent, symlinked activation component) is returned as invalid_params, telling the caller its path argument is invalid when the environment is the problem.
R10 | src/tools/finding_tests.rs:90-102 | the four copies of the db_path/open/publish block (61-73, 90-102, 124-136, 144-156) make six pairs; the ledger anchors only the three pairs against 61-73. Missing pairs: 90-102 vs 124-136, 90-102 vs 144-156, 124-136 vs 144-156.
R12 | src/tools/finding.rs:237 | the same path-in-warn leak as the confirmed :234 entry: the file_not_indexed message (and the empty-file :255 and line_end :262 arms) embeds the caller-supplied file path, and log_completion logs error.message at warn for every failed call.
R12 | src/tools/project_control.rs:180 | the config-write error embeds conf_path.display() (an absolute path under the project root); it is returned as err() and logged verbatim by server_runner::log_completion at warn.
R12 | src/tools/project_control.rs:217 | the try_exists error embeds activation.display() (an absolute path); it is returned as err() and logged verbatim by server_runner::log_completion at warn.

### Chunk 9
R1 | src/tools/leads_tests.rs:209 | `try_run(...).expect_err("unknown mechanic is rejected")` panics if the call unexpectedly succeeds; the same expect-family panic the ledger counts at other lines, but this one is absent from the ledger (likely because the grep matched `.expect(` only).
R1 | src/tools/leads_tests.rs:397 | `try_run(...).expect_err("a page pinned to a superseded revision is refused")` is an omitted expect-family panic; only the following `.expect` at 398 is reported.
R1 | src/tools/leads_page_tests.rs:79 | `try_run(asked).expect_err(...)` panics on an unexpected success and the whole file has no R1 entries in the ledger.
R1 | src/tools/leads_page_tests.rs:105 | `try_run(huge).expect_err("an offset above i64::MAX must fail")` omitted.
R1 | src/tools/leads_page_tests.rs:121 | `count_of(&[], "unwrap_calls").expect_err(...)` omitted; the ledger reports no panic constructs in this file.
R1 | src/tools/report_tests.rs:273 | `build_report(...).expect_err("malformed language")` omitted; only the `panic!` at 275 is reported, though both panic on the unexpected branch.
R10 | src/tools/leads_page.rs:104 | 10-line window 104-113 is duplicated verbatim at 147-156: the `Prepared` assembly shared by `full` and `compact` (`returned`/`total`/`files_total`/`files_covered`/`page_file_counts`/`limited_by`/`rows_bytes`/`})`/`}`). Nine code lines plus one blank; below the literal 10-code-line reading but a shareable constructor.
R10 | src/tools/report_tests.rs:392 | 10-line window 392-401 is duplicated verbatim at 414-423: `insert_indexed_file` trailing args + loop close + `transaction()` + `build_report()` setup in the two parser-failure-total tests. Nine code lines plus one blank.

### Chunk 10
R1 | src/tools/query_tests.rs:71 | `response.rows[0]["byte_len"]` is a data-dependent index with no preceding length assertion. The chunk R1 detector greps only unwrap/expect/panic/unreachable/todo/unimplemented, so data-dependent indexing in test scope is absent from the ledger; this is a systemic gap across the chunk test files, not one site.
R1 | src/tools/query_tests.rs:128 | `after.rows[0]["n"]` indexed with no in-function length guard (same pattern at 230, 243, 247, 253, 263).
R1 | src/tools/query_page_tests.rs:209 | `page.rows[0]["big"]` indexed with no length assertion; unguarded indexing also at 276/280 and 321/325.
R1 | src/tools/query_clip_tests.rs:43 | the `first_string` helper indexes `response.rows[0][column]` with no bounds guard in the helper; callers do not always assert `rows.len()` first.
R1 | src/tools/structure_tests.rs:131 | `response.matches[0].text` indexed with no preceding `matches.len()` assertion.

### Chunk 11
R2 | src/bin/check_docs_drift/test_file_count.rs:31 | `find_map(... .parse::<usize>().ok())` discards parse Results; strict R2 flags `.ok()` on a Result (used as a scan predicate, so lower confidence).
R5 | src/bin/check_docs_drift/main.rs:94 | `read`'s "cannot find {relative}" Err branch has no test; module_map/test_file_count tests always supply the document, so only the consumers' failure push is covered.
R5 | src/bin/check_performance/bench.rs:35 | `run_fresh` error arms (:35 stale-output cleanup, :43 cargo bench invoke, :45 non-success status) have no test asserting the exact error; no test calls `run_fresh`.
R5 | src/bin/check_performance/record.rs:39 | `record::write` create-dir and write error arms (:39, :42) have no test; only `baseline_json` is tested.
R5 | src/bin/check_producer_coverage/scan.rs:29 | `production_sources`/`collect` error arms (:29 not-a-directory, :44 read_dir, :47 entry, :61 file read) have no test; no test calls `production_sources`.
R5 | src/bin/check_mcp_surface/surface.rs:18 | `build`'s serialization `Err` arm has no test (documented cannot-happen guard).
R5 | src/bin/check_no_duplicates/test_names.rs:20 | `read_dir_entries` per-entry error arm has no test; the suite covers only directory and file read errors.
R7 | src/bin/check_performance/estimates.rs:46 | every `estimates.json` is read whole with no size cap (the recursive walk at :82 is flagged; the file read is not).
R7 | src/bin/check_performance/estimates.rs:103 | the baseline file is read whole with no size cap.
R7 | src/bin/check_version_consistency/manifests.rs:83 | the four manifest/version files are read whole with no size cap.
R7 | src/bin/check_mcp_surface/main.rs:101 | the committed snapshot is read whole with no size cap.
R7 | src/bin/check_docs_drift/main.rs:97 | ARCHITECTURE.md/MODULE_MAP.md are read whole with no size cap.
R7 | src/bin/check_producer_coverage/main.rs:135 | the declaration file is read whole with no size cap.
R7 | src/bin/check_governance/codeowners.rs:17 | CODEOWNERS is read whole with no size cap.
R5 | src/bin/check_performance/main.rs:83 | `run`'s "no baseline at ..." (:83) and "no criterion estimates found" (:90) error arms have no test; the tests exercise only the pure helpers.

### Chunk 12
R2 | tests/stdio_protocol.rs:182 | `self.child.try_wait().is_ok_and(|status| status.is_some())` turns an `io::Error` from `try_wait` into "not exited yet"; the poll failure is never reported. The adjacent `let _ = child.kill()` / `let _ = child.wait()` are reported, but this discarded `Result` is not.
R1 | benches/common/mod.rs:54 | `Mix::below` computes `self.next() % bound as u64` with no zero guard — the same latent modulo-by-zero class as the confirmed line 62. All current call sites pass the literal `100`, so it is latent, not currently reachable.

