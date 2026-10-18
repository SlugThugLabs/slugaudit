# SlugAudit code-quality audit: HEAD 7fcffd8, 2026-10-18

Files reviewed 255 of 255 (all read in full; one coverage gap was found mid-audit and closed).
Scope split across 6 auditors with no overlap; every cited line was re-opened before it was kept.
Full ledger, one violation per line: `.planning/audit_2026-10-18_ledger.tsv` (762 rows).

## Inventory
- Root crate `slugaudit-mcp-rust` 1.0.10: lib + 10 bins (`slugaudit` + 9 dev-only `check_*` gates) + 5 benches; 5 integration test files.
- Separate fuzz workspace `slugaudit-mcp-rust-fuzz`: 3 targets.
- 255 `.rs` files, 46,234 lines in total. Per-file counts are in `inventory.tsv`.

| Class | Files | Total lines | Code lines |
|---|---|---|---|
| shipped (src/, non-test, non-bin) | 132 | 21,593 | 14,357 |
| dev-only (src/bin non-test) | 31 | 3,022 | 2,185 |
| tests (src/*_tests.rs, tests.rs) | 77 | 19,014 | 15,263 |
| tests/ | 5 | 1,210 | 1,210 (wc) |
| benches | 6 | 904 | 904 (wc) |
| fuzz targets | 4 | 491 | 491 (wc) |

## Detectors

| Detector | Command | Raw result |
|---|---|---|
| Static analysis | `cargo clippy --all-targets --all-features --locked -- -D warnings` | 0 warnings, 0 errors |
| Tests | `cargo test --lib --bins --tests --all-features --locked` | 903 passed, 0 failed, 1 ignored (10k soak, on demand) |
| Licence/deps | `cargo deny check advisories bans sources licenses` (root + fuzz) | 0 issues; 1 warning (unmatched `libfuzzer-sys` exception on root) |
| Unused deps | `cargo machete` (root + fuzz) | 0 |
| Coverage | `check_coverage` (cargo llvm-cov) | 89.84% lines (gate 83%) |
| Complexity | `rust-code-analysis-cli` 0.0.25 | 2,675 fns; 73 over 50 lines (39 shipped); 11 with CC > 15 (4 shipped). INFORMATIONAL per owner decision D-13 |
| Fast check | the 7 `check_*` gates | all PASS |

## Rule table
Count = verified violations (ledger rows; a row can bundle several undocumented fields of one type). Exempt and documented items are listed in the ledger but not counted.

| Rule | Detector / method | Count | Target | Pass/Fail | Not counted |
|---|---|---|---|---|---|
| R1 No panics | rg + read | 12 (3 tools, 9 core) | 0 | Fail | 81 R1-exempt rows (test code) |
| R2 No silent failures | rg + read | 123 (29 dev-only) | 0 | Fail | 27 R2-documented |
| R3 No unsafe / waivers | rg | 6 | 0 | Fail | 17 R3-documented (print_stdout); 0 `unsafe` |
| R4 File size (project bands) | check_source_limits | 3 (1 shipped: stale exception on `store/connection.rs`) | 0 | Fail | 2 informational (tests/ and benches/ not gated) |
| R5 Failure-path tests | grep tests per error | 153 (53 dev-only) | 0 | Fail | n/a |
| R6 Truthful messages | read | 16 | 0 | Fail | n/a |
| R7 Bounded inputs | read | 38 | 0 | Fail | n/a |
| R8 Public API docs | read + grep | 195 rows | 0 | Fail | n/a |
| R9 Needless allocations | read | 26 | 0 | Fail | n/a |
| R10 Duplicated logic | 10-line window hash + read | 24 | 0 | Fail | 1 false positive dropped |
| R11 Clean dependencies | cargo deny + machete | 1 (deny warning) | 0 | Fail (warning only) | n/a |
| R12 Structured failure logs | read | 25 | 0 | Fail | n/a |

There are also 13 correctness observations outside R1–R12 (`OBS`/`RX` rows): gate-accuracy and bench-validity issues, plus stale doc references.

Dropped in verification: 1 R5 disproved, 1 R10 false positive, 3 R2 + 1 R3 + 26 R8 false hits inside string fixtures, 1 R12 doc-drift-only. Line numbers on 7 more were corrected rather than dropped.

## Top-10 high-risk findings (end-user impact)
1. R7/OBS `src/tools/structure.rs:161`: the agent's `language` string goes to `tree_sitter_language_pack::get_language`. The pack's default `download` feature is enabled (`Cargo.toml:80`, pack 1.18.0 `lib.rs:193`), so an unknown name fetches a network manifest, and a listed one is downloaded and loaded as a native library into the server. **Verified by the lead in the dependency source.**
2. R7 `src/evidence/calls/macro_calls.rs:70`: the macro-nesting cap (64) and the tree-walk cap (512) multiply, so stack depth is not bounded. A 29 KB file of nested `m!(((...)))` **aborts the server (stack overflow, uncatchable)** in a release build with a 2 MiB thread. Reproduced by the auditor. Any cloned dependency containing such a file kills the MCP server.
3. OBS `src/sync/revision.rs:270`: the first publish binds one SQL variable per file. Bundled SQLite allows 32,766, so **a project with more than 32,766 indexable files can never finish its first index**. Verified by the lead in code; the limit was probed by the auditor. `manifest.rs:92` and `reconcile/queries.rs:16` have the same issue for large reconciles.
4. R1 `src/tools/query_exec.rs:57`, `structure_exec.rs:36`: `Instant::now() + Duration` with unclamped env limits. A large `SLUGAUDIT_MAX_*_SECS` panics every `query`/`structure` call.
5. R2 `src/sync/sweep.rs:198`: any stat error is treated as "deleted", so a transient permission or IO error **removes a real file from the index**. The comment at :191 says the opposite.
6. R2 `src/evidence/calls.rs:32,35`, `macro_calls.rs:73-80`, `normalize.rs:112,115`: a failed parser setup or parse returns empty with "not capped", so the file is reported complete while call/symbol evidence is missing. This is silent truncation.
7. OBS `src/sync/publish_diff.rs:108`: edges are re-resolved only for re-indexed files. Adding a file does not fix earlier unresolved imports in unchanged files (stale dependency graph). Read from code, untested.
8. R7/R2 `src/watch/scope.rs:37`, `watch/manager.rs:339`: a full tree walk runs under the manager mutex with no deadline. A slow or huge filesystem blocks every tool call, and events arriving meanwhile are dropped with no log.
9. R12 `src/main.rs:43`: logging is initialised only for `serve`. Every warn in install/update/connect/disconnect is discarded, so a failed binary-staging cleanup leaves no trace.
10. R6 `src/connect_exec.rs:262`, `src/tools/project_control.rs:156`: `disconnect` prints "Done." when nothing was removed (no HOME, CLI missing, or detection-only clients), and `enable` reports `enabled` after silently dropping an unknown `profile`.

Next in line (in the ledger):
- fail-open symlink and network-filesystem checks (`activation.rs:54`, `connection.rs:90`, `netfs.rs:82,172`);
- `config.json` values unvalidated, and `"profile":"default"` discards the whole config;
- `SHA256SUMS` read under the 150 MB cap from the same origin, with no host check on update URLs (`update.rs:150`, `update_fetch.rs:47`);
- `.gitignore` read with no size cap (`activation.rs:218`, `ignore_rules/build.rs:129`);
- absolute paths leaked through `error = %e` in 12+ warns;
- dev-only: `check_coverage NaN` / `check_performance NaN` pass CI, fuzz targets can run green while testing nothing (`fuzz/fuzz_targets/support.rs:200`), and `check_docs_drift` doesn't require new files in the module map.

## Could not check
- R12 path leaks were inferred from dependency `Display` impls (notify 8.2.0, ignore 0.4.33), not from captured log output.
- macOS-only code (`netfs.rs` mount/sysctl) was read but not run.
- rmcp transport request-size limits were not read. They decide how much weight the per-field input limits carry.
- SQLite GLOB worst-case time for `leads`/`structure` patterns was not measured.
- Whether indexing (not only `structure`) triggers a parser download for exotic extensions (`parse/language.rs`, `get_parser`) was not traced.
- R5 relied on grepping test sources, so a test that asserts an error through unusual wording could be missed.
- `curl --max-filesize` enforcement on chunked bodies, and client CLIs' behavior on duplicate `mcp add`, were not tested.
- R4 for tests/ and benches/: the project gate does not scan them, so they were measured informally.
