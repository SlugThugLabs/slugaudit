# Module Map

This document is the exhaustive manifest of all source files in the SlugAudit codebase under `src/`.
It is checked for drift by the `check_docs_drift` gate.

For the system architecture, subsystem boundaries, data flow, and design invariants, see [ARCHITECTURE.md](ARCHITECTURE.md).

```
src/
├── main.rs                   binary entry: parse args, then serve MCP over stdio
├── lib.rs                    crate root: module declarations + crate-wide lint policy
├── cli.rs                    Command enum, parse_args, usage text
├── server.rs                 MCP surface: tool contracts, INSTRUCTIONS, tool router
├── server_runner.rs          tool-call execution: semaphore, spawn_blocking, progress
├── resources.rs              static MCP resources (cookbook, payload keys, schema)
├── progress.rs               ProgressEvent/ProgressSink bridge from sync to a renderer
├── ignore_rules.rs           single source of truth for excluded paths
├── ignore_rules/             submodule directory of the file module above
│   └── build.rs              per-directory matcher assembly for IgnoreRules::build
├── error_codes.rs            stable machine-readable failure codes + remedies
├── util.rs                   cross-module helpers (lock_or_recover, Deadline, SESSION_ID)
├── menu.rs                   interactive setup menu
├── prompt.rs                 bounded line input for the setup prompts
├── menu_render.rs            ANSI rendering helpers for the setup menu
├── install.rs                stable binary installation into ~/.slugthug
├── install_place.rs          staged-binary placement shared by install and update
├── update.rs                 self-update: fetch release, atomically replace the binary
├── update_fetch.rs           network + version helpers for self-update
├── connect.rs                register/remove this binary with a target agent
├── connect_agents.rs         supported agents and editors registry
├── connect_cli.rs            per-client `mcp add`/`remove` argv grammar and execution
├── connect_exec.rs           registration policy: dispatch, config merging, refusals
├── connect_guidance.rs       user-facing instructions, provenance and docs links
│
├── tools/                    MCP tool handlers — thin orchestration
│   ├── mod.rs                ToolCounters, shared credential predicate, exports
│   ├── context.rs            ensure_synced + revision verification
│   ├── context_transactions.rs  with_verified_read / with_verified_write
│   ├── report.rs             automatic project snapshot
│   ├── report_completeness.rs  the report's index-completeness block
│   ├── report_edges.rs       the report's inbound/outbound edge totals
│   ├── query.rs              arbitrary read-only SQL with budgets
│   ├── query_exec.rs         query execution: single-statement proof, budgets, byte-budgeted page, exact total
│   ├── page_size.rs          effective page size (request or default, clamped to the ceiling)
│   ├── query_value.rs        SQLite row/value → JSON with a per-value cap
│   ├── query_clip.rs         clipping an oversized first row's TEXT/BLOB cells, with substr continuations
│   ├── query_clip_tests.rs   clipping walks (5 MB text, blob, units) and line functions through query
│   ├── structure.rs          tree-sitter pattern match: contract, validation, targets
│   ├── structure_exec.rs     query execution under match cap + time budget
│   ├── structure_source.rs   source loading + multi-file and snippet helpers
│   ├── leads.rs              unranked mechanical observations
│   ├── leads_mechanics.rs    the leads mechanic catalogue
│   ├── leads_query.rs        the SQL each leads mechanic compiles to
│   ├── leads_page.rs         one response assembled: counts, a row page, or the compact enumeration
│   ├── leads_compact.rs      the compact mechanic/file/line row shape over one mechanic or all
│   ├── finding.rs            the single write tool
│   ├── finding_read.rs       session-gated finding query
│   ├── project_control.rs    enable/disable a project
│   ├── health.rs             operational snapshot
│   └── health_database.rs    health's read-only database read
│
├── sync/                     detection and persistence of what changed on disk
│   ├── mod.rs                public exports
│   ├── manager.rs            SourceSyncManager: state, watcher ownership, observability
│   ├── manager_orchestration.rs  ensure_current: resolve, open/recover, rebuild tail
│   ├── manager_health.rs     sync_by_health: the WatcherHealth dispatch
│   ├── manager_meta.rs       metadata row, session purge, publish_from_scratch
│   ├── sweep.rs              the stat sweep — the freshness backstop
│   ├── sweep_reconcile.rs    apply the sweep's findings under compare-and-swap
│   ├── reconcile/            dirty/deleted reconciliation
│   │   ├── mod.rs            re-exports + MAX_BARRIER_LOOPS
│   │   ├── error.rs          ReconcileError
│   │   ├── report.rs         ReconcileReport
│   │   ├── options.rs        ReconcileOptions (budget + ignore rules)
│   │   ├── pipeline.rs       per-path dirty-file loop
│   │   ├── barrier.rs        event-barrier sync loop
│   │   └── queries.rs        SQL the pipeline reads
│   ├── discovery.rs          filesystem walk with extension/limit filters
│   ├── hash.rs               BLAKE3 content hashing
│   ├── sample.rs             read-file-with-budgets
│   ├── sample_batch.rs       parallel batch sampling loop
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
│   ├── analyze.rs            tree-sitter parse + evidence extraction for one file
│   ├── classify.rs           path-derived authored/generated/vendored file class
│   └── race_hook.rs          test-only race-injection hook
│
├── graph/                    import evidence → dependency_edges
│   ├── mod.rs                public entry point (resolve captured Import evidence)
│   ├── reference.rs          ImportReference (pre-resolution extraction)
│   ├── resolve.rs            generic path-arithmetic utilities
│   ├── resolve_rust.rs       Rust crate::/super::/self:: resolution
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
│   ├── calls.rs              Rust call-site walker
│   └── generic_imports.rs    generic import extraction
│
├── watch/                    filesystem watching
│   ├── mod.rs                public exports
│   ├── manager.rs            WatchManager: notify lifecycle + event dispatch
│   ├── event_filter.rs       scope watch-dir diff + per-path event class
│   ├── state.rs              WatchState — thread-safe per-project state
│   ├── types.rs              WatcherHealth + ProjectWatchState
│   ├── scope.rs              which directories are watched
│   └── path.rs               relative-path normalization
│
├── project/                  project resolution + activation
│   ├── mod.rs                exports: resolve_project, enable/disable
│   ├── root.rs               ProjectRoot
│   ├── activation.rs         activation-directory marker read/write
│   ├── activation_lock.rs    exclusive database lock held while disable removes it
│   ├── database_path.rs      where this project's project.db lives
│   └── config.rs             in-repo config (.planning/slugaudit/config.json)
│
├── store/                    SQLite connection boundary
│   ├── mod.rs                open_read_write / open_read_only / discard_corrupt_database
│   ├── connection.rs         typed StoreError + symlink/permission/ATTACH guards
│   ├── netfs.rs              network-filesystem rejection
│   ├── sql_functions.rs      line_count / line_range scalar SQL functions registered on every connection
│   ├── sql_functions_tests.rs line-function edge cases
│   ├── migrations.rs         schema versioning (CURRENT_SCHEMA_VERSION)
│   ├── schema.sql            canonical schema
│   └── test_capture.rs       test-only tracing capture helper (owns PERMISSIVE_BACKSTOP)
│
├── parse/                    tree-sitter language-pack boundary
│   ├── mod.rs                PACK_VERSION + re-exports
│   └── language.rs           language detection + parser availability
│
├── model/                    contracts shared by parsing, storage, and tools
│   ├── mod.rs                re-exports
│   ├── limits.rs             ResourceLimits + process_limits()
│   ├── limits_tests.rs       limits unit tests
│   ├── profile.rs            Lean / Audit / Adaptive profiles
│   ├── profile_tests.rs      profile unit tests
│   ├── system_ram.rs         host memory detection
│   ├── parser.rs             parser-availability / parse-outcome enums
│   ├── evidence.rs           evidence kind + payload model
│   ├── source.rs             source-text model
│   └── span.rs               byte/line span types
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
