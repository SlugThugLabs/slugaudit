# 2026-10-18 audit ledger — triage against `main` at `f9544ca`

The ledger `.planning/audit_2026-10-18_ledger.tsv` was written against `7fcffd8`.
Every row the ledger marked `open` (484 rows) was re-checked against the current
code by five independent workers, using the row's description and current
`path:line`, not the old line numbers.

## Counts

| Rule | Handled | Still open | Not a violation | Needs decision |
|---|---:|---:|---:|---:|
| R1 | 0 | 0 | 0 | 0 |
| R2 | 0 | 0 | 0 | 0 |
| R3 | 0 | 0 | 0 | 0 |
| R4 | 1 | 2 | 2 | 0 |
| R5 | 4 | 149 | 0 | 0 |
| R6 | 4 | 12 | 0 | 0 |
| R7 | 12 | 22 | 2 | 2 |
| R8 | 3 | 192 | 0 | 0 |
| R9 | 2 | 24 | 0 | 0 |
| R10 | 0 | 23 | 2 | 0 |
| R11 | 0 | 0 | 1 | 0 |
| R12 | 2 | 17 | 6 | 0 |
| **Total** | **28** | **441** | **13** | **2** |

The ledger states were updated from `open` to `handled 2026-10-19 (backlog triage)`,
`still open (verified 2026-10-19)`, `not a violation (triage 2026-10-19)`, or
`needs decision (2026-10-19 triage)`. The per-row evidence is below and in the
ledger.

## Handled (28)

These were fixed before this triage and are now marked handled. The resource-wave
rows (`b199277`, `dd87904`, `94726ab`, `4d50e18`, `7d87f04`, `f9544ca`) are
included; the rest are the earlier R1/R2/R3 or gate fix rounds.

- **271 R4** `src/store/connection.rs:1` — `cargo run --bin check_source_limits -- --root /home/slugaudit` now reports src/store/connection.rs at 234 code lines (200-300 band), so the approved-by=slugthug annotation at connection.rs:1 is required, not stale; the file grew via 5b35572.
- **291 R5** `src/bin/check_no_duplicates/commit_subjects.rs:53` — test `a_failing_git_log_reports_its_stderr` (commit_subjects.rs:134-148, added f3f7d25) drives the failing git-log branch (:43-49) and asserts the stderr-bearing message.
- **323 R5** `src/bin/check_version_consistency/tests.rs:129` — f3f7d25 strengthened `version_consistency_unreadable_file_is_an_error_not_a_verdict` (check_version_consistency/tests.rs:124-132) to assert the error names CHANGELOG.md, produced by manifests.rs:81-84.
- **357 R5** `src/sync/hash.rs:21` — src/sync/hash.rs:258-269 `hash_file_reports_a_missing_path_as_a_read_error` asserts HashError::Read; added by 94726ab. (The post-open read-loop site :57-62 and the reconcile pass-through remain untested — see row 371.)
- **423 R5** `src/update_fetch_tests.rs:42` — src/update_fetch_tests.rs:63-69 `compare_versions_error_names_the_unparseable_version_and_keeps_the_cause` asserts the message names the bad version and source() is present; :42-43 remain is_err-only, but the error contract is pinned.
- **426 R6** `benches/memory.rs:98` — Fixed by f3f7d25: benches/memory.rs:98-107 panics with the real cause when Linux peak_rss_kib returns Err (:29-47), and Ok(None) only comes from the non-Linux stub (:50-53), so "not linux" is now accurate.
- **429 R6** `src/bin/check_no_duplicates/main.rs:37` — Fixed by f3f7d25: main.rs:63-66 now eprintln!s every entry in `failures`, including the git-invocation failures pushed by commit_subjects.rs:39-47.
- **440 R6** `src/tools/project_control.rs:182-190` — Fixed by 563745b: project_control.rs:135-148 now rejects an unrecognized profile with invalid_params before any side effect; project_control_tests.rs:337 asserts "unknown profile `strict`".
- **441 R6** `src/tools/project_control.rs:202-208` — Fixed by 563745b: disable now uses try_exists and maps a stat error to an error (project_control.rs:214-217) instead of emitting already_disabled.
- **442 R7** `src/bin/check_coverage.rs:47` — `parse_threshold` (src/bin/check_coverage.rs:142-157) rejects non-finite and out-of-range values; test `a_non_finite_or_out_of_range_threshold_is_rejected` (src/bin/check_coverage.rs:196); fixed by f3f7d25.
- **445 R7** `src/bin/check_performance/args.rs:32` — Finite/non-negative check at args.rs:31-42; test `the_cli_rejects_a_non_finite_or_negative_threshold` (src/bin/check_performance/tests.rs:251); fixed by f3f7d25. (`--bogus` still reports as a non-numeric threshold, but the gate fails closed with exit 2.)
- **450 R7** `src/evidence/calls/macro_calls.rs:70` — Explicit heap-stack scan (macro_calls.rs:83-105) with MAX_MACRO_DEPTH 64 and MAX_WALK_DEPTH truncation; fixed by 5f066bc; tests `a_rust_macro_nested_past_the_macro_bound_is_reported_partial_not_aborted`, `a_rust_file_of_nested_paren_heavy_macros_is_partial_on_a_small_stack`, `a_deeper_nested_macro_variant_is_partial_on_a_small_stack` (src/evidence/normalize_tests.rs:86,379,387).
- **458 R7** `src/project/activation.rs:218` — Read through `read_file_capped_bytes(..., MAX_FOREIGN_TEXT_BYTES)` at activation.rs:262 (dd87904); tests `an_oversized_gitignore_is_reported_and_left_byte_identical` and `a_gitignore_exactly_at_the_cap_is_still_inspected` (src/project/activation_tests.rs:426,455).
- **463 R7** `src/sync/hash.rs:21` — `hash_file` reads in 64 KiB chunks through `take(max_bytes+1)` (hash.rs:39-85; 94726ab); tests `hash_file_exactly_at_the_cap_succeeds` and `hash_file_one_byte_over_the_cap_reports_the_observed_size` (hash.rs:221,234).
- **464 R7** `src/sync/manifest.rs:92` — One JSON array parameter via `PATH_SET_SUBQUERY` (manifest.rs:74, used at :96-125; dbc2c72); test `the_manifest_hash_excludes_every_path_of_a_set_beyond_the_sqlite_variable_limit` (src/sync/revision_tests.rs:73).
- **465 R7** `src/sync/reconcile/queries.rs:16` — One JSON parameter via `PATH_SET_SUBQUERY`, plus MAX_PATH_LOOKUP_BATCH 4096 (queries.rs:10-46; dbc2c72/94726ab); tests `stored_hashes_are_found_for_a_path_set_beyond_the_sqlite_variable_limit` and `stored_hashes_are_found_in_every_lookup_batch` (queries.rs:81,97).
- **466 R7** `src/sync/revision.rs:270` — `invalidate_stale_findings` binds one JSON array (revision.rs:263-282; dbc2c72); tests `stale_findings_are_invalidated_when_touched_paths_exceed_the_sqlite_variable_limit` and `a_first_publish_of_more_files_than_the_sqlite_variable_limit_completes` (revision_tests.rs:33,119).
- **467 R7** `src/sync/sample.rs:82` — `sample_file` now uses `util::read_file_capped_bytes` after the stat (sample.rs:82-97; util.rs:104-121; 94726ab/b199277); tests `sample_file_reports_a_file_over_the_cap_as_too_large` (sample_tests.rs:231) and `capped_byte_read_reports_an_oversized_file_without_reading_it_whole` (util_tests.rs:35).
- **469 R7** `src/tools/leads.rs:72,76,104` — `int_bind` refuses offsets above i64::MAX (leads_query.rs:74-88), `limit` 0 clamps to 1, and `page_bytes` is resolved/clamped by page_size::resolve. Tests `an_offset_sqlite_cannot_hold_is_refused_in_both_modes_not_bound_as_no_limit` (leads_page_tests.rs:98), `a_zero_limit_is_clamped_to_one_row` (leads_tests.rs:194), `leads_page_bytes_above_the_ceiling_is_clamped_and_reported` (leads_page_tests.rs:45).
- **477 R7** `src/util.rs:66` — `read_file_capped_bytes` pre-stats and reads through `take(cap+1)` (util.rs:104-127; b199277), so growth and special-file reads are bounded; tests `capped_byte_read_reports_an_oversized_file_without_reading_it_whole` and `capped_byte_read_returns_a_file_exactly_at_the_cap` (util_tests.rs:35,24). Caveat: opening a FIFO can still block before the bounded read starts.
- **478 R7** `src/watch/scope.rs:37` — The scope walk is cap-bounded by `max_watch_entries`/`max_watch_depth` (scope.rs:89-105) and computed outside the manager lock (scope.rs:23-26; manager.rs:230,366; 7d87f04); tests `a_count_capped_scope_walk_is_reported_and_keeps_the_root`, `a_depth_capped_scope_walk_stops_at_the_limit_and_flags_it` (scope_tests.rs:70,97), `the_scope_walk_runs_without_holding_the_manager_lock` (manager_event_tests.rs:387).
- **480 R8** `benches/common/mod.rs:30` — Fields are private now (`benches/common/mod.rs:25-28`); commit f3f7d25 removed the `pub` field construct, so no public doc contract remains (the public `log` is documented at :31-32).
- **500 R8** `src/connect.rs:35` — `prefer_slugthug_binary` now has a full doc at `src/connect.rs:35-40`; commit 345f390 added it.
- **513 R8** `src/evidence/normalize_builders.rs:8` — `flatten_structure` now carries a `///` doc at :8-11; commit 5f066bc added it.
- **699 R9** `src/watch/manager.rs:288` — Restructured by 7d87f04: apply_scope_locked now takes the new WatchScope by value (scope.rs:198-265), borrows the old scope only for watch_dir_changes (:220), and assigns `info.scope = Some(scope)` at :265 — no clone of the watch-dir set remains.
- **700 R9** `src/watch/manager.rs:337` — Restructured by 7d87f04: handle_event takes the owned Event and apply_event iterates `&event.paths` (manager.rs:459-461); no `event.paths.clone()` remains.
- **745 R12** `src/model/system_ram.rs:48` — The warn now carries `causes = %causes.join("; ")` (system_ram.rs:49-52), with every read/parse cause collected at :27-47 (5b35572); no longer prose-only and the cause is kept.
- **758 R12** `src/sync/sweep.rs:202` — Stat failures now match `Err(error)` and log `error = %error` beside the redacted path_id (sweep.rs:238-242; 6eda70f); classification is covered by `a_stat_failing_with_something_other_than_not_found_leaves_the_file_unverifiable_not_gone` (sweep_tests.rs:467).

## Still open — production rules (100)

R4/R6/R7/R9/R10/R11/R12. These are the rows worth acting on first.

- **269 R4** `src/bin/check_source_limits/main.rs:13` — main.rs:12-14 still says "≥300 is a hard failure"; main.rs:105 prints `(>{ceiling}; hard failure)` with ceiling = PRODUCTION_CEILING 300 (counter.rs:50-52), so a 320-line file is reported unrescuable even though counter.rs:69-83 lets 301-350 pass with a dated human exception.
- **270 R4** `src/bin/check_source_limits/scanner.rs:106` — scanner.rs:106,118 set has_code on every literal byte, so each physical line of a multi-line raw string counts (pinned by tests.rs:58-65) while AGENTS.md:234-236 says literals are excluded; scanner.rs:108-109 skips an escaped newline without finish_line, undercounting a `\`-continuation.
- **427 R6** `src/bin/check_docs_drift/main.rs:78` — main.rs:78-81 still says a missing document "exits with code 2", but read() returns Err(()) at :94-95 and callers push a failure (module_map.rs:17-19, test_file_count.rs:10-12); main exits 1 at :63-65.
- **428 R6** `src/bin/check_docs_drift/test_file_count.rs:22` — The failure text at test_file_count.rs:22 still contains the two runs of 14 literal spaces ("so the              count this check ...").
- **430 R6** `src/connect.rs:50` — connect.rs:61-65 still prints "already registered ... Updating connection..." before acting, while run_installed_cli does only run_add with add_args (connect_exec.rs:123-131, connect_cli.rs:42-69) — no remove precedes the add, so a duplicate-rejecting client fails after the "Updating" promise. (No client's duplicate behaviour was tested, as the row notes.)
- **431 R6** `src/connect.rs:60` — connect.rs:71-73 still prints "Verify with: <cli> mcp list" for every agent with a cli — including warp, whose `oz` is detection-only (connect_agents.rs:393), and the bob/pi/opencode file-fallback path taken when the CLI is absent (connect_exec.rs:92-104).
- **432 R6** `src/connect_agents.rs:340` — is_connected (connect_agents.rs:341-354) trusts check_connected's substring scan for "slugaudit" (connect_cli.rs:38) and goose's whole-file contains (connect_agents.rs:360-364), so a `slugaudit-dev` entry reads as connected.
- **433 R6** `src/connect_cli.rs:93` — run_add prints only stderr on failure (connect_cli.rs:112-120) and discards the client's captured stdout, while ConnectError::AddFailed at cli.rs:151 still says "see the output above".
- **434 R6** `src/connect_exec.rs:262` — disconnect_agent (connect_exec.rs:271-289) returns Ok(()) when home is None, removes only configs it wrote (detection-only claude/codex/grok excluded at :274-281) and runs the CLI remove only when cli_on_path (:282-287); connect.rs:77-83 then prints "Done."
- **435 R6** `src/install.rs:225` — path_config builds `quoted` from display() and interpolates it unescaped into `fish_add_path {quoted}` and `export PATH="{quoted}:$PATH"` (install.rs:220-234); spaces break fish and `"`/`$`/newline land verbatim, after which :199 prints "Added SlugAudit to your PATH".
- **436 R6** `src/menu_render.rs:96` — menu_render.rs:96 still formats binary.display() into a JSON string with no escaping, so a path containing `"` or `\` produces invalid JSON presented as a config entry.
- **437 R6** `src/sync/manager_health.rs:101` — manager_health.rs:93-105 returns on a failed drain before record_full_publish at :117, although publish_full at :82-87 already landed; the counter documented at manager.rs:146-149 and reported by health under-reports exactly then.
- **438 R6** `src/tools/finding.rs:171` — The "finding recorded" info log at finding.rs:171-176 fires inside insert_finding (called from the write closure at :91) before with_verified_write commits at context_transactions.rs:33.
- **439 R6** `src/tools/leads_query.rs:24-31 and src/tools/structure_source.rs:71-75` — compile_pattern (leads_query.rs:24-32) and structure's file_filter (structure_source.rs:71-77) wrap a bare pattern as `*pat*` but still run it as GLOB, so `[`/`]` stay metacharacters; docs at leads.rs:56-60 and server.rs:247-248 call it a substring match, yet `app/[id]/page.tsx` matches nothing.
- **443 R7** `src/bin/check_coverage.rs:84` — Unbounded `read_to_string` remains at src/bin/check_coverage.rs:91, src/bin/check_docs_drift/main.rs:97, src/bin/check_governance/codeowners.rs:17, src/bin/check_mcp_surface/main.rs:101, src/bin/check_no_duplicates/test_names.rs:80, src/bin/check_performance/estimates.rs:46 and :103, src/bin/check_producer_coverage/scan.rs:60 and main.rs:135, src/bin/check_source_limits/main.rs:89, src/bin/check_version_consistency/manifests.rs:83 (dev-only, repo-file inputs).
- **444 R7** `src/bin/check_no_duplicates/commit_subjects.rs:16` — `commit_range` (commit_subjects.rs:68-76) interpolates the env value into `{base}..HEAD` and passes it to `git log` with no `--` and no leading-`-` validation (commit_subjects.rs:22-23); `SLUGAUDIT_CHECK_COMMITS_BASE=--output=/tmp/x` is parsed as a git option.
- **446 R7** `src/bin/check_source_limits/counter.rs:105` — `path.is_dir()` recursion with no depth limit or cycle guard remains at counter.rs:101-107, check_no_duplicates/test_names.rs:56-66, check_producer_coverage/scan.rs:48-53, check_docs_drift/test_file_count.rs:66-68, check_performance/estimates.rs:80-83; a symlink cycle under src/ recurses until stack overflow.
- **447 R7** `src/cli.rs:35` — `parse_args` (cli.rs:35-52) and `UnknownAgent(String)` (cli.rs:145) echo caller text with no length cap; only the OS ARG_MAX bounds it.
- **448 R7** `src/connect_cli.rs:14` — `check_connected` (connect_cli.rs:17-29) and `run_add` (connect_cli.rs:103-110) use `Command::output()` with no timeout and no output cap; the connect menu calls `is_connected` for every detected agent (connect.rs:96) and again on selection (connect.rs:113).
- **451 R7** `src/evidence/generic_imports.rs:132` — The whole-statement `source` payload is still built uncapped at generic_imports.rs:167-168; `max_payload_bytes_per_item` is applied only later at src/sync/sample.rs:260.
- **452 R7** `src/evidence/normalize.rs:28` — `extract` (normalize.rs:28-70) still materializes every pack item before any cap; caps run later at src/sync/sample.rs:218-267 (called via src/sync/analyze.rs:85).
- **453 R7** `src/ignore_rules/build.rs:129` — `GitignoreBuilder::add` (ignore 0.4.33 has no byte/line/file cap) is called directly at build.rs:143; dd87904 bounded only the activation `.gitignore` read (src/project/activation.rs:262).
- **454 R7** `src/install.rs:182` — The old `interactive_prompt` is gone, but `install.rs:134` now calls `prompt::confirm`, whose `ask_yes_no` still uses an uncapped `read_line` (src/prompt.rs:53-62), bypassing the 256-byte `read_prompt_line` cap (prompt.rs:5-17).
- **455 R7** `src/install.rs:9` — SLUGTHUG_HOME/HOME/SHELL/APPDATA/PATH remain unvalidated (install.rs:8-16,34-38,203-211); `bin_dir.display()` is interpolated into `export PATH="{quoted}:$PATH"` at install.rs:220,228,233, so a hostile SLUGTHUG_HOME containing `"`/`$()` injects shell text into the rc file.
- **457 R7** `src/model/system_ram.rs:27` — `/proc/meminfo` is still read with unbounded `read_to_string` (system_ram.rs:28). Kernel-trusted and negligible, but the construct the row names remains.
- **460 R7** `src/server.rs:446` — `resource_not_found` still formats `request.uri` unbounded (server.rs:444-448; message at :447).
- **461 R7** `src/store/netfs.rs:182` — macOS `/sbin/mount` runs via `Command::output()` with no timeout and unbounded stdout (netfs.rs:203-205); sysctl likewise (src/model/system_ram.rs:62-65). A hung/stale mount blocks database open.
- **462 R7** `src/store/netfs.rs:77` — `/proc/self/mountinfo` is still read with unbounded `read_to_string` (netfs.rs:86). Kernel-trusted and negligible, but the construct remains.
- **468 R7** `src/tools/finding.rs:211-223` — title/description/severity/category are still checked only by `chars().count()` after the whole string arrives (finding.rs:211-226); the byte pre-checks added by 563745b cover only `path` and `file` (finding.rs:186-192).
- **470 R7** `src/tools/leads_query.rs:90 and leads_compact.rs:129 and structure_source.rs:76` — Caller GLOB still runs with no SQLite progress handler: leads row/count queries (leads_query.rs:90-115,169-204), leads_compact row/count queries (leads_compact.rs:111-121,142-160) and structure's `path GLOB` source query (structure_source.rs:63-95); only the Tree-sitter structure execution has a deadline (structure_exec.rs:38).
- **472 R7** `src/tools/structure.rs:62-66,217-221` — `validate_request` caps file/language/pattern but not `start_after_file` (structure.rs:213-227); it is bound into SQL at structure_source.rs:79-81.
- **473 R7** `src/tools/structure.rs:67-70,158` — structure does not cap `expected_revision` (structure.rs:213-245), while query/leads cap it at 256 bytes (query.rs:151-156; leads.rs:344); `guard_expected_revision` echoes the value (context.rs:125-131).
- **474 R7** `src/update.rs:150` — `verify_checksum` still `read_to_string`s the whole SHA256SUMS (update.rs:149-151); its download cap is 150 MB (update_fetch.rs:112).
- **475 R7** `src/update_fetch.rs:47` — `from_json` accepts any `browser_download_url` host/prefix (update_fetch.rs:57-79) and `from_tag` interpolates `tag_name` (update_fetch.rs:36); only HTTPS is enforced (update_fetch.rs:101-103).
- **476 R7** `src/update_fetch.rs:87` — `curl_get` still relies on `--max-filesize 10000000` with `--max-time 30` (update_fetch.rs:87) and buffers stdout; mid-transfer size enforcement for chunked bodies is unproven.
- **479 R7** `src/watch/state.rs:37` — `mark_dirty`/`mark_deleted` still insert into unbounded HashSets (state.rs:37-55); 7d87f04 added no cap, so a noisy unignored directory grows them between syncs.
- **675 R9** `src/evidence/calls.rs:84` — walk still calls call_parts (building owned callee_text/callee_name/caller) at calls.rs:97, and emit_call builds key=format!("call:{counter}") at :129 before seen.insert decides the node is a duplicate at :131.
- **676 R9** `src/evidence/normalize.rs:185` — normalize.rs:207 still allocates SymbolKind::Other("Field".to_owned()) only to Debug-format it at :226 (`format!("{sym_kind:?}")`).
- **677 R9** `src/graph/resolve_rust.rs:138` — resolve_rust.rs:139-144 and :282 still build `vec![format!(...)]` candidate lists only to pass them as a slice.
- **678 R9** `src/graph/resolver/types.rs:63` — pick() still collects all candidates into `Vec<&String>` (resolver/types.rs:62-66) just to read len() and matches[0].
- **679 R9** `src/ignore_rules/build.rs:63` — group_files_by_dir still calls entry(parent.to_path_buf()) (build.rs:69) and pushes file.clone() into both Vecs (:71,:73) instead of borrowing.
- **680 R9** `src/install.rs:225` — install.rs:220 still allocates `quoted = bin_dir.display().to_string()` and only formats it into the line at :224/:228/:233.
- **681 R9** `src/menu_render.rs:92` — menu_render.rs:92 and :96 still build a temporary String per line with `text.push_str(&format!(...))`.
- **682 R9** `src/project/activation.rs:243` — ensure_gitignore still builds `entry = format!(...)` and `addition = format!(...)` as two Strings at activation.rs:289-290.
- **683 R9** `src/store/connection.rs:70` — discard_corrupt_database still builds each sidecar name with `format!("{}{}", path.display(), suffix)` (connection.rs:79), which is lossy for non-UTF-8 project paths, so the real -wal/-shm files can survive a corruption discard.
- **684 R9** `src/sync/manifest.rs:84` — compute_manifest_hash still collects rows into Vec<(String,String)> then immediately into_collects into the BTreeMap at manifest.rs:105-111 and :120-129.
- **685 R9** `src/sync/publish_diff.rs:108` — changed_paths is still a HashSet<String> cloned from every changed path (publish_diff.rs:108-119), and deletions clone the paths again at :120-124.
- **686 R9** `src/sync/publish_diff.rs:74` — publish_diff.rs:74-75 still calls `current_refs.clone()` for compare() and then moves/iterates current_refs into aggregate_manifest_hash.
- **687 R9** `src/sync/revision_edges.rs:31` — known_paths still clones the full discovered set, inserts every stored path, and collects into Vec<String> (revision_edges.rs:31-37); revision.rs:195-196 then rebuilds a HashSet<&str> from it.
- **688 R9** `src/sync/sample.rs:101` — sample_file still does `text.to_owned()` on the validated UTF-8 bytes (sample.rs:104-108), and revalidate still calls sample_file for a full read+copy only to compare one hash (revalidate.rs:67-68).
- **689 R9** `src/tools/finding.rs:117,160` — insert_finding still calls `session_id().to_string()` twice per call, at finding.rs:117 and :160.
- **690 R9** `src/tools/finding.rs:90` — finding() still clones synced.revision_id at finding.rs:90 only to pass `&revision_id` at :91.
- **691 R9** `src/tools/finding_read.rs:97-109` — read_session_findings still builds an owned String SQL plus Vec<String> params (finding_read.rs:97-109).
- **692 R9** `src/tools/leads_query.rs:172-174 and :206-208` — rows_for still does `Value::Text(glob.to_owned())` at leads_query.rs:184 and files_covered at :218, after which glob is only used for `.is_some()`.
- **693 R9** `src/tools/project_control.rs:124` — project_control.rs:124 still allocates `inner.path.as_deref().unwrap_or(".").to_string()` and immediately borrows it.
- **694 R9** `src/tools/query_value.rs:45 and src/tools/query_exec.rs:182` — sql_value_to_json still clones every TEXT cell at query_value.rs:112, and query_exec.rs:220 still builds row_to_json from `&values` in the common fitting-row path; 4d50e18 only avoids conversion for rows whose lower bound already exceeds the page.
- **695 R9** `src/tools/report.rs:149,151` — report() still clones synced.revision_id at report.rs:149 and clones it again inside the FnOnce at :151.
- **696 R9** `src/tools/structure_exec.rs:165` — structure_exec.rs:178-180 still calls serde_json::to_vec(&found)?.len() per match instead of util::serialized_len.
- **697 R9** `src/tools/structure_page.rs:40-44` — require_captures still builds the message as `"..".to_owned() + ".." + ".."` at structure_page.rs:42-46.
- **698 R9** `src/tools/structure_source.rs:76,80` — file_filter still uses `sql.push_str(&format!(...))` at structure_source.rs:76 and :80.
- **714 R10** `src/connect.rs:12` — run_connect (connect.rs:12-21) and run_disconnect (:24-33) are still the same find_agent/UnknownAgent match differing only in the callee.
- **715 R10** `src/connect_tests.rs:54` — connect_tests.rs:57-78 (cursor) and :82-101 (zed) still repeat the same ~22-line connect/is_connected/read/disconnect sequence.
- **716 R10** `src/evidence/calls.rs:166` — node_span is still duplicated three times: calls.rs:213-226, generic_imports.rs:176-191, normalize.rs:320-335.
- **717 R10** `src/evidence/calls_tests.rs:105` — calls_tests.rs:106-113 and :119-127 are still identical callee_name=="unwrap" count-1 assertion blocks.
- **718 R10** `src/evidence/generic_imports.rs:199` — Second copy of node_span still at generic_imports.rs:176-191 (see row 716).
- **719 R10** `src/evidence/generic_imports_tests.rs:128` — The 18-row language/source table at generic_imports_tests.rs:128-147 is still duplicated verbatim at normalize_grammar_tests.rs:137-156.
- **720 R10** `src/evidence/normalize.rs:305` — Third copy of node_span still at normalize.rs:320-335 (see row 716).
- **721 R10** `src/evidence/normalize_grammar_tests.rs:64` — Three near-identical loops still at normalize_grammar_tests.rs:56-71, :76-88 and :92-107, differing only in the language/source table.
- **722 R10** `src/ignore_rules/build.rs:82` — compile_ignore_matcher (:88-97) and compile_gitignore_matcher (:127-140) still repeat the same build().map_err(warn).ok() block.
- **723 R10** `src/install_tests.rs:233` — path_config tests at install_tests.rs:233-284 and config_home tests at :108-165 still repeat the same temp_env::with_vars scaffold.
- **724 R10** `src/model/limits.rs:130` — The repeated `if let Some(val) = parse_override(...)` block is now 14 copies at limits.rs:163-205 (it grew, not shrank).
- **725 R10** `src/model/profile.rs:107` — Audit (profile.rs:117-137) and Adaptive (:145-165) still repeat the same 8 field lines plus the identical EvidenceLimits block.
- **727 R10** `src/store/netfs.rs:54` — log_rejected_network_filesystem still carries a copied "Refuses to open a database ..." paragraph at netfs.rs:13-25 that belongs to reject_network_filesystem (:53-72), so the log helper's doc describes the wrong action.
- **728 R10** `src/sync/publish_race_tests.rs:98` — publish_race_tests.rs:150-166 and :225-241 are still identical setup blocks (temp project, a.rs, temp db, bootstrap publish, both channels).
- **729 R10** `src/sync/sweep_tests.rs:311` — sweep_tests.rs:314-322 and :369-376 are still identical setup blocks (project, lib.rs, default manager, first sync, Healthy state).
- **730 R10** `src/tools/ (8 files)` — `const MAX_PATH_BYTES: usize = 32 * 1024;` still appears in finding_read.rs:15, health.rs:43, report.rs:74, finding.rs:16, project_control.rs:15, leads.rs:39, query.rs:15, structure.rs:17, and messages still hardcode 32768 (e.g. report.rs:146).
- **731 R10** `src/tools/finding.rs:227-240 vs src/tools/structure_source.rs:10-33` — fetch_file and fetch_source still repeat the same query_row + `format!("{file}: {error}")` + FileNotIndexed payload shape.
- **732 R10** `src/tools/leads.rs:343 <-> src/tools/query.rs:144` — The path-bytes and expected_revision-bytes validation blocks are still duplicated at leads.rs:330-352 and query.rs:149-160 (same messages).
- **733 R10** `src/tools/leads_page.rs:101-111 vs :145-155` — full() and compact() still end with the same 11-line Prepared construction (leads_page.rs:102-111 and :145-154) and repeat the through/complete/files_covered logic at :92-100 vs :137-143.
- **734 R10** `src/tools/leads_query.rs:166-193 vs src/tools/leads_compact.rs:111-129` — rows_for still repeats compile_pattern/fetch=limit+1/binds/prepare/query_map/fill_page in leads_query.rs:169-205 and leads_compact.rs:103-130.
- **735 R10** `src/tools/leads_query.rs:24-31 vs src/tools/structure_source.rs:71-75` — The glob/substring compile logic is still copied between leads_query.rs:24-32 and structure_source.rs:71-77 rather than shared; the docs even say "the same way structure does".
- **736 R10** `src/update_fetch.rs:85` — curl_get (:85-101) and download (:105-130) still repeat the curl Command build, HTTPS_ONLY, ToolSpawn map_err and stderr-to-Network handling.
- **737 R10** `src/update_tests.rs:25` — The expected-hash setup (sha256sum + split_whitespace + write SHA256SUMS) is still repeated at update_tests.rs:27-44 and :63-79.
- **741 R12** `src/ignore_rules/build.rs:107` — Global gitignore error (build.rs:44-49) and `.git/info/exclude` error (build.rs:110-115) log `error = %error`; ignore 0.4.33 `Error::WithPath` Display prints `path.display()` (crate src/lib.rs:338-340), leaking the absolute home/project path, contrary to rule (c) DECISIONS.md:1341-1342.
- **742 R12** `src/ignore_rules/build.rs:132` — `add_files` logs `path = %rel(...)` but also `error = %err` (build.rs:141-150); `GitignoreBuilder::add` wraps errors `.with_path(path)` (ignore-0.4.33 src/gitignore.rs:405-408), whose Display prints the absolute path, contradicting the comment at build.rs:25-28.
- **744 R12** `src/main.rs:43` — `init_logging()` still runs only after `run_command` returns `Serve` (main.rs:34-43; function at :155), so install/update/connect/disconnect/menu warnings (install_place.rs:86, connect_exec.rs:223, util.rs:50-54) are discarded.
- **747 R12** `src/project/config.rs:85` — `load_or_default` warns on every call (config.rs:82-97) and is called per tool call (tools/mod.rs:130), per publish (publish_cas.rs:55) and per reconcile (reconcile/options.rs:32); the root is now redacted, but the parse error text can still echo config values.
- **751 R12** `src/sync/manager_health.rs:179` — `drain_events` warns `error = %error` (manager_health.rs:191-197) where ErrorData embeds ReconcileError, whose transparent Discovery/Hash/Sample variants carry absolute PathBufs (reconcile/error.rs:14-24; discovery.rs:44-48; sample.rs:23-31); root_id is redacted but the error text is not.
- **752 R12** `src/sync/manager_health.rs:92` — The post-verification drain warning uses `error = %error` (manager_health.rs:93-105); the SyncError/ReconcileError message embeds absolute paths.
- **753 R12** `src/sync/manager_meta.rs:237` — `map_publish_error` warns `error = %error` (manager_meta.rs:234) after `publish_log::log_outcome` already warned the same PublishError (publish_log.rs:16); both carry absolute paths from DiscoveryError/SampleError.
- **754 R12** `src/sync/manager_orchestration.rs:127` — `reason = %reason` (manager_orchestration.rs:124-129) contains `different project root ({stored_root})`, built at :113-115, i.e. the previous project's absolute root.
- **755 R12** `src/sync/publish_attempt.rs:57` — `warn_skipped` logs a redacted `path_id` but `reason = %file.reason` (publish_attempt.rs:57-63); discovery/sample skip reasons are `DiscoveryError::Read`/`SampleError::TooLarge` Display text containing the absolute path (discovery.rs:44-48; sample.rs:23-31).
- **756 R12** `src/sync/publish_log.rs:16` — Publish failure warns `error = %error` with no redaction (publish_log.rs:16); PublishError Display embeds DiscoveryError/SampleError path text.
- **757 R12** `src/sync/sweep.rs:169` — Sweep walk errors warn `error = %error` with no path field (sweep.rs:188-193); ignore::Error Display includes the offending absolute path.
- **759 R12** `src/sync/sweep_reconcile.rs:58` — `sweep_failed` warns `error = %error` (sweep_reconcile.rs:53-61) where the ErrorData message is built from `ReconcileError` at sweep_reconcile.rs:106-112, embedding absolute paths.
- **760 R12** `src/util.rs:207` — `lock_or_recover` still logs the prose-only "mutex was poisoned; recovering inner value" with no key-value fields or mutex identity (util.rs:268-272).
- **761 R12** `src/watch/manager.rs:149` — A failed root watch logs `root_id = redacted` but `error = %error` (manager.rs:186-193); notify::Error Display appends the paths.
- **762 R12** `src/watch/manager.rs:313` — Unwatch/watch failures still log `error = %error` beside redacted ids: manager.rs:282-285 (unwatch root) and scope.rs:242-255 (unwatch/watch per directory).
- **763 R12** `src/watch/manager.rs:93` — Watcher-callback errors warn `error = %error` with no path redaction (manager.rs:116-119); watcher-creation failure likewise (manager.rs:137-140); notify::Error Display appends the affected paths.
- **764 R12** `src/watch/scope.rs:47` — Scope walk errors warn `error = %error` with no path field (scope.rs:107-119); the depth-limit inspect failure logs a redacted path_id but `error = %error` (scope.rs:149-159); ignore::Error Display includes the absolute path.

## Still open — R5 failure-path tests (149)

All still open: no current test asserts the exact error/behavior named by the
row. The classes are the store/network fail-closed paths, activation/config
I/O, sample/hash failure paths, input-size caps, bare `internal_error` arms,
the coverage gate's own FAIL path, and dev-only gate failures. Rows are in the
ledger with the original descriptions and current-code evidence.

## Still open — R8 documentation (192)

All still open except the three handled rows above. The current
`rustc -W missing_docs` run reports 239 warnings / 212 unique sites; the
remaining rows are missing field/variant docs and public items that are
unreferenced outside their module. Two doc-contract defects are high value:
`src/project/config.rs` says the profile value `default` is accepted (it is
not), and `src/model/limits.rs` says invalid env overrides are silently
ignored (they warn).

## Not a violation (13)

- **268 R4** `benches/common/mod.rs:1` — Explicitly INFORMATIONAL and benches/ is outside check_source_limits' scope: main.rs:59,67 walks only `src/`, so no annotation is required.
- **272 R4** `tests/stdio_protocol.rs:1` — Explicitly INFORMATIONAL and tests/ is outside the gate's scope: main.rs:59,67 walks only `src/`.
- **449 R7** `src/evidence/calls.rs:35` — Parse-time interruption is a documented accepted limitation: DECISIONS.md:405-407 ("A single pathological file's tree-sitter parse is not interruptible mid-parse"); parse call at src/evidence/calls.rs:30-35.
- **471 R7** `src/tools/structure.rs:161` — Runtime grammar download is an explicit owner decision: DECISIONS.md:1480-1496 ("Keep runtime grammar downloads (accepted risk)"); `get_language` at structure.rs:161-162, and f9544ca reverted the static bundle.
- **726 R10** `src/server.rs:224` — The repeated signature/dispatch shape at server.rs:225,259,282,306,329,350,377,399 is mandated by the rmcp #[tool_router] macro; the row itself records "n/a: low" and no DRY form exists without leaving the framework.
- **738 R10** `tests/stdio_protocol.rs:166` — Explicitly INFORMATIONAL and below the 10-line duplication threshold (0 exact 8-line normalized windows), so there is no gate-relevant finding.
- **739 R11** `deny.toml:31` — Expected warning, not a violation: the libfuzzer-sys/NCSA exception is deliberate and documented (deny.toml:24-31); the root graph excludes fuzz/, and quality.yml:166-171 runs the same policy against fuzz/Cargo.toml where the exception matches. cargo-deny warnings do not fail the gate.
- **740 R12** `src/connect_exec.rs:213` — One-shot temp-file cleanup warning with a stated deliberate reason (connect_exec.rs:221-223), covered by the path-logging rule (b), DECISIONS.md:1337-1342. Separately it is discarded when run via `connect` (row 744).
- **743 R12** `src/install_place.rs:86` — One-shot cleanup diagnostic with a stated deliberate reason (install_place.rs:78-87), covered by rule (b), DECISIONS.md:1337-1342; discarded without a subscriber today (row 744).
- **746 R12** `src/project/activation.rs:161` — Activation-dir tightening logs the raw path with a stated deliberate reason (activation.rs:188-198), explicitly covered by rule (b), DECISIONS.md:1337-1342.
- **748 R12** `src/store/connection.rs:214` — Store guard refusal, raw path with a stated reason (connection.rs:249-263), covered by rule (b), DECISIONS.md:1337-1342. The caller line (manager_orchestration.rs:86-99) uses a redacted database_id and `StoreError::Permissions` Display carries no path, so the raw path appears once.
- **749 R12** `src/store/connection.rs:97` — Store guard refusal, raw path with a stated deliberate reason (connection.rs:110-134), covered by rule (b), DECISIONS.md:1337-1342; `StoreError::Symlink` Display (connection.rs:24) carries no path.
- **750 R12** `src/store/netfs.rs:42` — Store guard refusal, raw path with a stated reason (netfs.rs:35-52), covered by rule (b), DECISIONS.md:1337-1342; the later log (manager_orchestration.rs:86-99) uses a redacted database_id and `StoreError::NetworkFilesystem` Display carries no path.

## Needs decision (2)

- **456 R7** `src/model/limits.rs:130` — 7d87f04 clamps only SQLite-bound `max_file_bytes`/`max_query_value_bytes` (limits.rs:163-164,184-187,215-227); all other overrides (limits.rs:166-205) still accept 0 and u64::MAX with no minimum or cross-field relation check. Owner call: reject vs clamp, and enforce `default_page_bytes <= max_query_response_bytes`, `max_total_import_bytes >= max_file_bytes`.
- **459 R7** `src/project/config.rs:67` — 7d87f04 clamps `max_file_bytes` in `apply_to_limits` (config.rs:107-117; test `a_config_file_above_sqlite_max_is_clamped`, config.rs:227), but `max_total_import_bytes` and `default_page_bytes` still accept 0/u64::MAX unvalidated (config.rs:118-123). Owner call on reject/clamp policy for a committed config from an untrusted repo.

Both are owner calls on limit validation: `0`/`u64::MAX` overrides and
cross-field relations (`default_page_bytes <= max_query_response_bytes`,
`max_total_import_bytes >= max_file_bytes`). `7d87f04` clamped only the
SQLite-bound values.

## Caveats

- Row 477 (`read_file_capped`) is marked handled: the read is now bounded by
  `take(cap + 1)`, but opening a FIFO can still block before the read starts.
- The R8 count uses the current lint as the authoritative missing-doc list;
  rows for doc contradictions and external references were checked by hand.
- No row was left unverified.
