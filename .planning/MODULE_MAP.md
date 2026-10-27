# Module Map

This document is the exhaustive manifest of all source files in the SlugAudit codebase under `src/`.
It is checked for drift by the `check_docs_drift` gate.

For the system architecture, subsystem boundaries, data flow, and design invariants, see [ARCHITECTURE.md](ARCHITECTURE.md).

```
src/
├── main.rs                   binary entry: parse args, then serve MCP over stdio
├── lib.rs                    crate root: module declarations + crate-wide lint policy
├── cli.rs                    Command enum, parse_args, CLI text output helpers
├── cli/                      submodule directory of the file module above
│   └── usage.rs              USAGE help text + styled usage() rendering
├── server.rs                 MCP surface: INSTRUCTIONS, dispatch, resource plumbing + router wiring
├── server/                   submodule directory of the file module above
│   └── handlers.rs           #[tool] handlers: wire titles, descriptions, annotations
├── server_runner.rs          tool-call execution: semaphore, spawn_blocking, progress
├── resources.rs              static MCP resources (cookbook, payload keys, schema)
├── progress.rs               ProgressEvent/ProgressSink bridge from sync to a renderer
├── ignore_rules.rs           single source of truth for excluded paths
├── ignore_rules/             submodule directory of the file module above
│   └── build.rs              per-directory matcher assembly for IgnoreRules::build
├── error_codes.rs            stable machine-readable failure codes + remedies
├── util.rs                   cross-module helpers (lock_or_recover, redaction, re-exports)
├── util/                     submodule directory of the file module above
│   ├── capped_read.rs        bounded reads of foreign text files
│   ├── style.rs              ANSI styling for the human-facing CLI
│   └── time.rs               process timestamps + cooperative deadlines
├── clock.rs                  wall-clock reads that log instead of silently zeroing (unix seconds, saturating millis)
├── probe.rs                  client-detection probes: path existence and CLI-on-PATH, logging every non-not-found failure
├── menu.rs                   interactive setup menu
├── prompt.rs                 bounded line input for the setup prompts
├── menu_render.rs            ANSI rendering helpers for the setup menu
├── install.rs                stable binary installation into ~/.slugthug
├── install/                  submodule directory of the file module above
│   ├── path.rs               shell PATH line selection, quoting, idempotent append
│   └── path_tests.rs         PATH-line shell quoting and idempotency
├── install_place.rs          staged-binary placement shared by install and update
├── update.rs                 self-update: fetch release, atomically replace the binary
├── update_fetch.rs           network + version helpers for self-update
├── connect.rs                register/remove this binary with a target agent
├── connect/                  submodule directory of the file module above
│   ├── config_file.rs        JSON config probe/merge/remove for register and disconnect
│   ├── config_file_tests.rs  removal truthfulness, atomic mode/symlink, typed I/O failures
│   └── registry_types.rs     client model types (Strategy, ConfigShape, Evidence, Dialect)
├── connect_agents.rs         supported agents and editors registry
├── connect_cli.rs            per-client `mcp add`/`remove` argv grammar and execution
├── connect_exec.rs           registration policy: dispatch, config merging, refusals
├── connect_guidance.rs       user-facing instructions, provenance and docs links
│
├── tools/                    MCP tool handlers — thin orchestration
│   ├── mod.rs                shared credential predicate, module declarations, exports
│   ├── tool_counters.rs      process-wide tool-call counters exposed by health
│   ├── context.rs            ensure_synced, revision verification, with_verified_read/write
│   ├── report.rs             automatic project snapshot: contract, completeness envelope, edge totals
│   ├── report_sections.rs    per-section SQL + response assembly for report
│   ├── report_completeness.rs  the report's index-completeness block
│   ├── report_first_call_tests.rs the first report after project_control on, with a real watcher
│   ├── query.rs              arbitrary read-only SQL with budgets
│   ├── query_exec.rs         query execution: single-statement proof, budgets, byte-budgeted page, exact total
│   ├── page.rs               the one paging contract (`Page`, `PageLimit`) every paged tool embeds
│   ├── paginate.rs           the one paging entry: new request syncs and materialises, a cursor only reads the stored result
│   ├── page_size.rs          effective page size (request or default, clamped to the ceiling)
│   ├── query_value.rs        SQLite row/value → JSON with a per-value cap
│   ├── query_clip.rs         clipping an oversized first row's TEXT/BLOB cells, with substr continuations
│   ├── query_clip_tests.rs   clipping walks (5 MB text, blob, units) and line functions through query
│   ├── query_error_tests.rs  sql_error on SQLite failures; refusals and value caps carry no code
│   ├── query_paged_tests.rs  query paged through the result store: cursor, snapshot, continuation
│   ├── golden_phase9_support.rs  shared golden helpers: fixtures, normalise, mask_paging (removes paging keys), check_case
│   ├── golden_phase9_leads_tests.rs  leads golden cases (counts, rows, compact, custom mechanic rows)
│   ├── golden_phase9_mechanic_tests.rs  mechanic add/list golden cases
│   ├── golden_phase9_query_tests.rs  query golden cases (values, clipping, paging)
│   ├── golden_phase9_structure_tests.rs  structure golden cases
│   ├── golden_phase9_finding_read_tests.rs  finding_read golden cases
│   ├── structure.rs          tree-sitter pattern match: contract, validation, targets
│   ├── structure_exec.rs     query execution under match cap + time budget
│   ├── structure_page.rs     byte paging of matches and the multi-file memory-window cursor
│   ├── structure_source.rs   source loading + multi-file and snippet helpers
│   ├── structure_parse_bounds_tests.rs max_parse_bytes skip, window-before-load and parse-timeout partials
│   ├── structure_language_tests.rs unknown_language naming the indexed languages
│   ├── structure_stored.rs   structure matches stored as a result and served by cursor
│   ├── structure_paged_tests.rs structure paged through the result store: both cursors, continuation
│   ├── leads.rs              unranked mechanical observations
│   ├── leads_mechanics.rs    the leads mechanic catalogue
│   ├── leads_covers.rs       each count's `covers`: the languages the mechanic read
│   ├── leads_language_tests.rs leads and covers over non-Rust call languages
│   ├── leads_query.rs        the SQL each leads mechanic compiles to
│   ├── leads_not_parsed_tests.rs files_not_parsed on every leads response and report's oversize count
│   ├── leads_page.rs         one response assembled: counts, a row page, or the compact enumeration
│   ├── leads_compact.rs      the compact mechanic/file/line row shape over one mechanic or all
│   ├── leads_custom.rs       registered mechanics in leads: counts and the streaming page cut
│   ├── leads_stored.rs       leads rows stored as a result and served by cursor
│   ├── leads_paged_tests.rs  leads paged through the result store: cursor, partial custom scans
│   ├── mechanic.rs           auditor-registered mechanics: add/remove/list contract
│   ├── mechanic_register.rs  add/remove: id, collision and cap rules beside the writes they guard
│   ├── mechanic_list.rs      the stored `list` result and its cursor continuation
│   ├── mechanic_catalog.rs   one call's catalogue: session, project_config, built-ins, and invalid entries
│   ├── mechanic_scan.rs      a registered mechanic's tree-sitter scan under one deadline
│   ├── mechanic_scan_cached.rs the per-revision cache in front of that scan: lookup, one pass for the misses, rows kept for paging
│   ├── mechanic_tests.rs     add/count/remove, collisions, refused queries and languages, ordering, first-report note
│   ├── mechanic_persist_tests.rs persist/reload, other keys kept, symlink refused, invalid entries reported and removed by index
│   ├── mechanic_scan_tests.rs which registered entries a leads call scans; the spent-budget error and its code
│   ├── mechanic_paged_tests.rs mechanic list paged through the result store
│   ├── mechanic_scan_stop_tests.rs a registered mechanic's scan stopped by the deadline: partial page and resume_after_file
│   ├── finding.rs            the single write tool: request/response contract and caps
│   ├── finding_insert.rs     validation + session-scoped insert-or-refresh of a finding
│   ├── finding_read.rs       session-gated finding query
│   ├── finding_read_rows.rs  finding_read row selection and conversion for the result store
│   ├── finding_read_paged_tests.rs finding_read paged through the result store
│   ├── project_control.rs    enable/disable a project: contract and handlers
│   ├── health.rs             operational snapshot: schema, process counters, DB read
│   └── health_phase.rs       health's per-project state derivation (no side effects)
│
├── sync/                     detection and persistence of what changed on disk
│   ├── mod.rs                public exports
│   ├── manager.rs            SourceSyncManager: state, watcher ownership, observability
│   ├── result_stores.rs      the process's lazily opened result store, one slot keyed by runtime dir
│   ├── manager_orchestration.rs  ensure_current: resolve, open/recover, recovery logs, rebuild tail
│   ├── manager_health.rs     sync_by_health: the WatcherHealth dispatch
│   ├── manager_health_support.rs  event barrier, post-sync revision read, sweep-error mapping
│   ├── manager_meta.rs       metadata row, session purge, publish_from_scratch
│   ├── sweep.rs              the stat sweep — freshness backstop with walk counters
│   ├── sweep_walk_tests.rs   fail-closed walk coverage for the stat sweep
│   ├── sweep_reconcile.rs    apply the sweep's findings under compare-and-swap
│   ├── reconcile/            dirty/deleted reconciliation
│   │   ├── mod.rs            re-exports + MAX_BARRIER_LOOPS
│   │   ├── error.rs          ReconcileError
│   │   ├── report.rs         ReconcileReport
│   │   ├── options.rs        ReconcileOptions (budget + ignore rules)
│   │   ├── pipeline.rs       per-path dirty-file loop
│   │   ├── barrier.rs        event-barrier sync loop
│   │   └── queries.rs        SQL the pipeline reads
│   ├── discovery.rs          filesystem walk with entry collection and filters
│   ├── hash.rs               hash types + in-memory digests (re-exports hash_file/stat_identity)
│   ├── hash_io.rs            bounded streaming hash_file + stat_identity
│   ├── sample.rs             read-file-with-budgets
│   ├── sample_batch.rs       parallel batch sampling loop
│   ├── sample_batch_tests.rs undeliverable-result abort for the batch loop
│   ├── publish.rs            drive a single publish
│   ├── publish_attempt.rs    one attempt: discover → sample → diff → commit
│   ├── publish_cas.rs        compare-and-swap retry primitive
│   ├── publish_diff.rs       before/after diff for diagnostics
│   ├── publish_log.rs        per-attempt tracing outcome
│   ├── revalidate.rs         re-check sampled files before the commit
│   ├── manifest.rs           manifest hash + change classification
│   ├── revision.rs           revisions/files/evidence writes (atomic current-pointer swap)
│   ├── revision_edges.rs     dependency_edges writes for upserted files
│   ├── revision_files.rs     files/evidence writes for one upserted file
│   ├── revision_skipped.rs   existence rows for discovered-but-skipped files
│   ├── revision_support.rs   stale-finding invalidation inside a publish transaction
│   ├── analyze.rs            tree-sitter parse + evidence extraction for one file
│   ├── analyze_results.rs    the ParseResult an analysis ends in, one constructor per outcome
│   ├── analyze_seam.rs       pack-extraction boundary + test-only failure/call-count seam
│   ├── analyze_bounds_tests.rs max_parse_bytes and parse_timeout outcomes of analyze
│   ├── classify.rs           path-derived authored/generated/vendored file class
│   └── race_hook.rs          test-only race-injection hook
│
├── graph/                    import evidence → dependency_edges
│   ├── mod.rs                public entry point (resolve captured Import evidence)
│   ├── reference.rs          ImportReference (pre-resolution extraction)
│   ├── resolve.rs            generic path-arithmetic utilities
│   ├── resolve_rust.rs       Rust crate::/super::/self:: resolution
│   ├── rust_paths.rs         Rust module-file candidates + segment-shortening walk
│   └── resolver/             generic + per-language resolvers
│       ├── mod.rs            entry point + dispatch
│       ├── generic.rs        LanguageResolver trait + generic resolver
│       ├── python.rs         Python relative imports
│       ├── js.rs             JS/TS import forms
│       ├── extract.rs        raw import statement → reference
│       ├── path_helpers.rs   path-arithmetic helpers
│       ├── registry.rs       resolver registry + public entry points
│       └── types.rs          resolution outcome model
│
├── evidence/                 evidence extraction + normalization
│   ├── mod.rs                neutral normalization of pack output
│   ├── normalize.rs          pack node → evidence
│   ├── normalize_builders.rs payload builders
│   ├── sql.rs                EvidenceKind ↔ SQL text mirror
│   ├── bindings.rs           variable-binding / field-declaration walker
│   ├── calls.rs              call dispatch + the Rust call-site walker
│   ├── calls/                submodule directory of the file module above
│   │   ├── site.rs           the one Call row builder every producer feeds
│   │   ├── macro_calls.rs    Rust macro token-tree re-parse for nested calls
│   │   ├── call_queries.rs   embedded per-language call queries (examples/calls/*.scm) + compile-once cache
│   │   ├── query_calls.rs    runs a call query: budgeted parse, capture → call site
│   │   └── query_calls_tests.rs per-language call fixtures with comment/string decoys
│   └── generic_imports.rs    generic import extraction
│
├── watch/                    filesystem watching
│   ├── mod.rs                public exports
│   ├── manager.rs            WatchManager: notify lifecycle + per-project state
│   ├── manager/              submodule directory of the file module above
│   │   ├── events.rs         lock-held event dispatch + deferred replay
│   │   └── scopes.rs         scope registration and refresh
│   ├── manager_callback_tests.rs  notify error callback: non-blocking Desynced marker
│   ├── errors.rs             static path-free categories for watcher failures
│   ├── event_filter.rs       scope watch-dir diff + per-path event class
│   ├── state.rs              WatchState — thread-safe per-project state
│   ├── types.rs              WatcherHealth + ProjectWatchState
│   ├── scope.rs              which directories are watched (pure walk + caps)
│   ├── scope_apply.rs        applies a computed scope under the manager lock
│   └── path.rs               relative-path normalization
│
├── project/                  project resolution + activation
│   ├── mod.rs                exports: resolve_project, enable/disable
│   ├── root.rs               ProjectRoot
│   ├── activation.rs         activation-directory marker read/write
│   ├── gitignore.rs          project-root .gitignore guard for activation
│   ├── gitignore_tests.rs    guard write-failure and file-type refusal tests
│   ├── activation_lock.rs    exclusive database lock held while disable removes it
│   ├── database_path.rs      where this project's project.db lives
│   ├── config.rs             in-repo config (.planning/slugaudit/config.json)
│   ├── config_cleanup.rs     stale config.json.<pid>.<n>.tmp removal on load, under a non-blocking try of config.lock
│   ├── config_cleanup_tests.rs stale temp removed once, live writer's kept, symlink not followed
│   ├── config_store.rs       the one write path of config.json: config.lock, read-modify-write, atomic replace
│   ├── config_store_tests.rs keys kept, unreadable refused, crash before rename, symlink re-check, lock wait, no lost update
│   ├── mechanics_config.rs   the versioned `mechanics` section of config.json: read, report, read-modify-write
│   ├── mechanics_config_tests.rs persisted section: other keys kept, nothing unreadable overwritten, bad entries reported
│   ├── mechanic_scan_cache.rs the byte-budgeted per-revision cache of registered mechanics' scan results
│   ├── session_mechanics.rs  the per-server in-memory mechanics and the one-time report note
│   └── config_tests.rs       config load and limit-mapping tests
│
├── store/                    SQLite connection boundary
│   ├── mod.rs                open_read_write / open_read_only / discard_corrupt_database
│   ├── connection.rs         typed StoreError + the open entry points
│   ├── connection_guards.rs  symlink/permission guards, ATTACH/PRAGMA authorizer, open flags, pragmas
│   ├── connection_guard_tests.rs read-only authorizer, PRAGMA allowlist and open-guard tests
│   ├── discard.rs            discard an unusable database only if it is still unusable under the write lock
│   ├── discard_tests.rs      lock-checked discard outcomes
│   ├── netfs.rs              network-filesystem rejection
│   ├── result_store/         immutable paged results: one unlinked SQLite file per result, registry, cursors
│   │   ├── admission_tests.rs      delivered/idle reclaim order, active never reclaimed, Full, TooLarge, budget invariant
│   │   ├── cell.rs                 Cell storage classes, NonFinite marks, StoredRow, Take, Materialize trait
│   │   ├── cursor.rs               ResultId and the strict v1 cursor codec
│   │   ├── cursor_tests.rs         cursor round trip, malformed variants, foreign nonce, expired, ordinal bound
│   │   ├── error.rs                typed ResultStoreError (source kept) and its wire-code mapping note
│   │   ├── fault.rs                test-only fault seam: full disk, failure between batches, incomplete meta
│   │   ├── fidelity_tests.rs       storage-class matrix, NaN and infinity marks, typed rows
│   │   ├── harness_tests.rs        shared fixtures: temp store, injected clock, descriptor and directory inspection
│   │   ├── lifecycle_tests.rs      self-abort, mid-write failure, full disk, process abort, sweep, threads
│   │   ├── mod.rs                  module docs, layout and invariants, re-exports
│   │   ├── owner.rs                results directory, owner lock and the startup sweep
│   │   ├── paging_tests.rs         page walks at batch boundaries, grace renewal, delivered, incomplete state
│   │   ├── platform.rs             Unix-only file helpers (private dir/file, lock file, same-inode check)
│   │   ├── reader.rs               read_page, read_cursor and read_typed over a stored result
│   │   ├── registry.rs             registry entries, classes, admission and reclaim planning
│   │   ├── result_file.rs          create-open-unlink a result file, pragmas read back, schema, size measure
│   │   ├── stats.rs                StoreStats snapshot
│   │   ├── store.rs                ResultStore: open, clock, cursor resolution, admission logging
│   │   ├── writer.rs               ResultWriter: batches in one transaction, meta last, size self-abort
│   ├── sql_functions.rs      line_count / line_range scalar SQL functions registered on every connection
│   ├── sql_functions_tests.rs line-function edge cases
│   ├── migrations.rs         schema versioning (CURRENT_SCHEMA_VERSION)
│   ├── schema.sql            canonical schema
│   └── test_capture.rs       test-only tracing capture helper (owns PERMISSIVE_BACKSTOP)
│
├── parse/                    tree-sitter language-pack boundary
│   ├── mod.rs                PACK_VERSION + re-exports
│   ├── budget.rs             one tree-sitter parse that stops at a deadline (shared by structure, mechanic scans, call queries)
│   ├── fetch_gate.rs         process-wide cool-down after a failed grammar fetch
│   └── language.rs           language detection + parser availability
│
├── model/                    contracts shared by parsing, storage, and tools
│   ├── mod.rs                re-exports
│   ├── limits.rs             ResourceLimits + process_limits()
│   ├── limits_tests.rs       limits unit tests
│   ├── limit_overrides.rs    zero-override refusal, parse clamp and the warn-once ledger
│   ├── limit_overrides_tests.rs zero refusal and warn-once tests
│   ├── profile.rs            Lean / Audit / Adaptive profiles
│   ├── profile_tests.rs      profile unit tests
│   ├── system_ram.rs         host memory detection
│   ├── parser.rs             parser-availability / parse-outcome enums
│   ├── evidence.rs           evidence kind + payload model
│   ├── source.rs             source-text model
│   ├── span.rs               byte/line span types and the per-file LineIndex
│   └── span_tests.rs         LineIndex equivalence with char_column
│
└── bin/                      development-only gate binaries
    ├── check_source_limits/  per-file code-line ceilings
    │   ├── main.rs           check loop + verdict printer
    │   ├── counter.rs        token-aware Rust code-line counter
    │   ├── scanner.rs        token scanner state machine
    │   └── approval.rs       exception-comment parser
    ├── check_no_duplicates/      duplicate commit subjects / test names
    │   ├── main.rs              args, verdict
    │   ├── commit_subjects.rs   git log subject dedupe
    │   └── test_names.rs        #[test] fn-name dedupe
    ├── gate_common/          shared by the rooted gates via #[path]; not a binary
    │   └── args.rs           --help / --root / switches argv contract, exit 2 on usage error
    ├── check_docs_drift/     documentation drift gate
    │   ├── main.rs           argv + failure printer + exit codes
    │   ├── canonical_docs.rs the canonical doc set must exist
    │   ├── module_map.rs     this module map must reference real paths
    │   └── test_file_count.rs  the `*_tests.rs` count claim must match
    ├── check_producer_coverage/  every evidence kind is constructed somewhere
    │   ├── main.rs           argv + gate verdict + exit codes
    │   ├── scan.rs           enum-variant parser + production-source scanner
    │   └── tests.rs          scanner rules + the verdict on this repo
    ├── check_mcp_surface/    the tools/list + resources/list snapshot gate
    │   ├── main.rs           argv + snapshot compare/record + exit codes
    │   ├── surface.rs        the live surface, derived from the server
    │   ├── diff.rs           structural snapshot diff with paths
    │   └── tests.rs          diff rules + the annotation/title table
    ├── check_governance/     governance-shell files + CODEOWNERS path gate
    │   ├── main.rs           argv + failure printer + exit codes
    │   ├── required_files.rs the fixed governance file set must exist
    │   ├── codeowners.rs     CODEOWNERS must reference only real paths
    │   └── tests.rs          the two checks' verdicts
    ├── check_version_consistency/  manifests, CHANGELOG, and pack pin agree
    │   ├── main.rs           argv + failure printer + exit codes
    │   ├── manifests.rs      version parsing + the three checks
    │   └── tests.rs          parser rules + the verdict on a fixture
    ├── check_coverage.rs     llvm-cov line-coverage gate
    └── check_performance/    criterion medians vs. the committed baseline
        ├── main.rs           run benches, compare, print verdict
        ├── estimates.rs      criterion estimate collection + baseline loading
        └── format.rs         time formatting
```
