# SlugAudit Rust Rewrite — Execution Plan

Status: implemented through Phase 12 with release-gate execution recorded
(2026-08-10); see §22 for the 2026-08-11 audit corrections.

Project root: `/opt/slugaudit-mcp-rust`

Reference checkout: `/opt/slugaudit-mcp`

The Python checkout is reference material only. It is not a dependency of the
Rust build and must not be modified as part of this plan. The old checkout may
be deleted later by the user after the Rust implementation is accepted.

## 1. Product definition

SlugAudit gives an AI fresh, searchable, trustworthy, relationship-aware
evidence about an arbitrary codebase while minimizing repetitive file reads and
token waste.

SlugAudit is not an autonomous bug detector. It must not decide that a pattern
is suspicious, label a finding, assign severity, recommend a fix, or imply that
an audit has already been completed. The AI receives evidence and performs the
reasoning.

The Rust implementation must provide:

- complete non-binary file indexing;
- bounded source retrieval;
- literal and regular-expression search;
- Tree-sitter language detection and parsing through the language pack;
- generic structural evidence for the full language-pack catalog;
- definitions, symbols, imports, exports, comments, docstrings, diagnostics,
  source spans, and syntax-aware chunks where the parser supplies them;
- a searchable file-to-file dependency graph;
- current-source freshness verification before every answer;
- SQLite persistence with one database per project;
- AI-reviewed finding persistence, without automated finding generation;
- compact responses designed for an AI caller, never for human reading;
- explicit evidence limitations so missing extraction is visible rather than
  mistaken for absence;
- exactly one human-facing control — enabling or disabling SlugAudit for a
  project — with every other behavior automated and reachable only through
  AI-invoked MCP tool calls;
- an immediate full import that starts the moment a project is enabled,
  without waiting for an AI to make the first tool call;
- fully automatic, mandatory freshness synchronization before every tool
  call regardless of whether the initial import has finished, with no manual
  sync, rebuild, or maintenance command exposed to a human or an AI.

The first release does not include:

- Neo4j;
- PostgreSQL;
- a second persistence backend;
- app-level SQL text validation/rewriting as a security boundary (raw,
  unrestricted, read-only SQL against a project's own database file is in
  scope — see the `query` tool in Task 7.3 — but its safety comes from a
  read-only connection, not from parsing query text);
- regex-based risk detection;
- autonomous bug classification;
- a language-specific extractor class for every grammar;
- backwards compatibility with the Python implementation;
- migration of existing Python databases;
- any human-facing command other than enabling/disabling a project (no
  manual sync, rebuild, changed-files, parsing, or database-maintenance
  command, for a human or for an AI);
- human-readability formatting requirements on any tool response.

## 2. Non-negotiable engineering constraints

These are acceptance criteria, not suggestions.

### 2.1 Toolchain

- Rust compiler: `1.97.1`.
- Edition: `2024`.
- Toolchain file: `rust-toolchain.toml`.
- Do not update the compiler during the rewrite.
- After the rewrite is accepted, compiler updates are a separate, deliberate
  change with a full validation run.

### 2.2 Source limits

- Every production `.rs` file targets fewer than 200 code lines. Code lines
  exclude blank lines, line comments, block comments, and documentation
  comments; executable statements, declarations, macro bodies, and generated
  source are counted according to the enforcement tool's report.
- A hand-written file from 200 through 300 code lines may be approved by the
  implementation agent only with a documented reason showing that splitting it
  would worsen coupling, duplicate logic, or obscure an indivisible state
  machine. The exception must record code-line count, alternatives considered,
  and the specific indivisible logic. A file over 300 code lines blocks
  implementation and requires the user's approval before work continues. No
  6,000-line class of file is acceptable.
- Tests may exceed 200 code lines only when a fixture table or integration
  scenario genuinely benefits from being co-located; prefer splitting tests as
  well.
- Generated files and vendored third-party files are excluded from the limit.
- The line-count check must be automated and must fail CI.
- A file may not be kept below 200 lines by hiding production logic in macros,
  giant inline constants, or generated source.

### 2.3 Safety

- SlugAudit source must contain no unsafe Rust.
- Apply `#![forbid(unsafe_code)]` to the crate.
- Third-party crates may contain internal unsafe code when required by
  Tree-sitter, SQLite, networking, or other FFI. That is not SlugAudit source
  and must be reported by `cargo geiger`, not incorrectly treated as a reason
  to reject the architecture.
- Never weaken safety checks to make a dependency compile.

### 2.4 Architecture

- One module has one primary reason to change.
- Tool handlers orchestrate; they do not own parsing, storage SQL, file
  walking, dependency resolution, or response serialization internals.
- The parser layer does not write to SQLite.
- The store layer does not read project files.
- The MCP transport does not contain product logic.
- Shared data structures live in one model layer.
- Repeated validation, hashing, span conversion, and response metadata logic
  must have one implementation.
- Cross-module contracts use typed Rust structs and enums.
- Errors are propagated or explicitly recorded; no `unwrap()` or `expect()` in
  production paths unless a review record proves the invariant is local,
  immutable, and impossible to violate.
- No global mutable state.
- No hidden network access during an ordinary audit query unless the language
  parser cache explicitly requires a missing grammar and the behavior is
  surfaced in the response metadata.

### 2.5 Evidence behavior

- Parsing is evidence collection, not judgment.
- A parser error is evidence and must include path, language, location where
  available, and parser error information.
- An unsupported extension is not silently treated as a successfully parsed
  source file.
- A grammar being present in the language pack does not justify language-
  specific assumptions that are not validated.
- Generic extraction must preserve raw spans and source locations.
- The AI must be able to distinguish:
  - no matching evidence;
  - parser unavailable;
  - parser failed;
  - parser succeeded but a field was not supplied;
  - file is not source-like and was indexed as content only.

### 2.6 Database

- SQLite is the only database in the Rust rewrite.
- One project owns one database file under its activation directory.
- All writes occur inside explicit transactions.
- Sync publishes a complete revision atomically.
- Readers never observe a half-written revision.
- Source content and derived evidence are replaced together for a changed file.
- Deleted files and their derived records are removed.
- Stale findings are invalidated by source hash.
- SQL uses parameters for values.
- Schema changes have explicit migrations and migration tests.

## 3. Target architecture

```text
src/main.rs
    process entry point only

src/server.rs
    stdio MCP transport and server lifecycle

src/tools/
    one small module per public tool contract — four tools total
    report.rs      automatic project snapshot (no query needed)
    query.rs       read-only SQL against the project's own database file
    structure.rs   Tree-sitter structural pattern matching
    finding.rs     the one write: persist an AI-reviewed conclusion

src/project/
    root validation, activation marker, database location

src/sync/
    discovery, ignore rules, hashing, manifest comparison, revision publish

src/parse/
    language-pack detection, parser invocation, result normalization boundary

src/evidence/
    typed neutral evidence records, span conversion, evidence serialization

src/store/
    schema, connection setup, repositories, transaction boundaries

src/search/
    bounded literal search, regex search, line-context retrieval

src/graph/
    import normalization, file matching, dependency-edge queries

src/model/
    shared request/response types and freshness metadata

tests/
    unit, contract, integration, fixture projects, protocol smoke tests
```

The exact module names may change during implementation, but ownership may not
be blurred. If a file approaches 200 lines, split by responsibility before
adding more behavior.

## 4. Standard validation commands

Run these from `/opt/slugaudit-mcp-rust`.

### Fast local gate

```bash
cargo fmt --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

### Full test gate

```bash
cargo nextest run --all-targets
cargo test --doc
```

### Coverage gate

```bash
cargo llvm-cov nextest --all-features --workspace --lcov --output-path lcov.info
cargo llvm-cov report --summary-only
```

Coverage is not accepted as the only quality measure. Critical contracts also
need direct integration tests and adversarial cases.

### Dependency and unsafe gate

```bash
cargo audit
cargo deny check
cargo geiger --all-features
```

`cargo geiger` is an inventory. Third-party unsafe may be present. SlugAudit
source unsafe is a hard failure.

### Mutation gate

```bash
cargo mutants --no-shuffle
```

Mutation testing begins after the core behavior exists. A surviving mutation
in freshness, deletion, hashing, parser-status reporting, or evidence query
logic blocks acceptance until the test is improved or the mutation is proven
irrelevant.

### Structural gate

```bash
find src -type f -name '*.rs' -print0 | xargs -0 wc -l
rg -n '\bunsafe\b|unwrap\(|expect\(' src
git diff --check
```

The final project must replace these exploratory commands with a checked-in
script that fails on production violations and excludes only documented test
or generated paths.

## 5. Phase 0 — Freeze the boundary and baseline the new project

Goal: establish the new project as an independent Rust build before adding
product behavior.

### Task 0.1 — Record the independent-project boundary

Files:

- `.planning/IMPLEMENTATION_PLAN.md`
- `README.md`
- `ARCHITECTURE.md`

Actions:

1. State that `/opt/slugaudit-mcp-rust` is the implementation project.
2. State that `/opt/slugaudit-mcp` is reference-only.
3. State that no Python database migration is planned.
4. State that there are no downstream users and no compatibility promise.
5. State that SQLite is the initial and only backend.

Validation:

1. `test -f README.md`.
2. `test -f ARCHITECTURE.md`.
3. `rg -n 'reference|SQLite|no.*compat|not.*auditor' README.md ARCHITECTURE.md`.
4. Confirm `git -C /opt/slugaudit-mcp status --short` shows no changes caused by
   the Rust project.

Failure condition: any plan text implies the old Python implementation must
remain a runtime dependency.

### Task 0.2 — Pin and validate the compiler

Files:

- `rust-toolchain.toml`
- `Cargo.toml`

Actions:

1. Pin `1.97.1`.
2. Require `rustfmt` and `clippy` components.
3. Keep edition `2024`.
4. Do not add a minimum compiler version that contradicts the toolchain pin.

Validation:

```bash
rustc --version
cargo metadata --no-deps --format-version 1
cargo fmt --check
cargo check
```

Expected result: compiler reports `1.97.1`; metadata reports edition 2024;
format and check pass.

### Task 0.3 — Add crate-wide unsafe prohibition

Files:

- `src/main.rs` or `src/lib.rs`

Actions:

1. Add `#![forbid(unsafe_code)]` at the crate root.
2. Do not add unsafe escape hatches.

Validation:

1. `cargo check` passes.
2. Add a temporary test branch locally that uses unsafe and confirm compilation
   fails; remove the temporary change.
3. `rg -n '\bunsafe\b' src` returns only the crate attribute if present.

### Task 0.4 — Add the production file-size gate

Files:

- `tools/check_source_limits.sh`
- `Cargo.toml` or CI configuration later

Actions:

1. Enumerate hand-written production Rust files under `src/`.
2. Count code lines separately from comments and blank lines using a Rust-aware
   line-counting tool such as `tokei`.
3. Fail if any ordinary production file reaches 200 code lines.
4. Permit 200–300 code-line files only with an explicit recorded reason,
   alternatives, and indivisible logic; reject files over 300 unless a user
   approval reference is present.
5. Exclude generated/vendor files only through explicit paths and report them.
6. Fail if SlugAudit source contains unsafe code through compiler-backed
   enforcement, not a raw text grep alone.
7. Report the exact violating path, code-line count, and exception status.
8. Keep the script deterministic and independent of Git state.

Validation:

1. Run the script against the current scaffold.
2. Create a temporary file with 199 code lines plus unlimited comments and prove
   it passes.
3. Create a temporary file with 200 code lines plus comments and prove it
   fails; remove the fixture.
4. Create a 200–300 code-line file with a documented reason and prove it is
   accepted; create a file over 300 and prove the gate blocks it until a user
   approval reference is present.
5. Run `git diff --check`.

## 6. Phase 1 — Define the typed evidence contract

Goal: establish the stable data model before storage, parsing, or MCP formatting.

### Task 1.1 — Define source identity and spans

Files:

- `src/model/source.rs`
- `src/model/span.rs`

Types:

- project-relative path;
- content hash;
- file size;
- modification timestamp when available;
- language-pack language name;
- byte span;
- one-based line and column span;
- parser status.

Validation:

1. Unit-test byte-to-line conversion for empty, single-line, multi-line, and
   Unicode source.
2. Unit-test that paths are project-relative and reject absolute paths.
3. Unit-test that a hash changes when one byte changes.
4. Run `cargo nextest run model`.
5. Run the file-size gate.

### Task 1.2 — Define neutral parser evidence

Files:

- `src/model/parser.rs`
- `src/model/evidence.rs`

Types:

- `ParserStatus` with explicit states for unavailable, downloaded, loaded,
  parsed, and failed;
- parser diagnostics;
- structural item;
- symbol definition;
- import;
- export;
- comment;
- docstring;
- chunk;
- raw/normalized evidence envelope.

Rules:

1. Every record carries a source span when the pack provides one.
2. Optional pack fields remain optional in the model.
3. Normalization never invents semantic certainty.
4. Unknown enum values become an explicit `Other(String)` representation.
5. Raw pack evidence is retained only under the defined per-file/per-record
   budgets, with truncation and provenance visible when the budget is reached.

Validation:

1. Serialize and deserialize every evidence type.
2. Test unknown `Other` values.
3. Test that parser failure serializes with path and diagnostic information.
4. Test that an empty result is distinguishable from parser unavailable.
5. Run `cargo test --all-targets` and `cargo clippy ... -D warnings`.

### Task 1.3 — Define freshness metadata

Files:

- `src/model/freshness.rs`
- `src/model/response.rs`

Fields:

- contract version;
- schema version;
- project identifier;
- revision identifier;
- manifest hash;
- parser-pack version;
- sync timestamp;
- freshness status;
- evidence completeness summary.

Validation:

1. Every public response type must be able to carry metadata.
2. Unit-test that missing revision metadata cannot be marked verified.
3. Unit-test that a parser-pack version change changes the freshness input.
4. Snapshot-test the compact JSON shape.

## 7. Phase 2 — Build the SQLite store

Goal: create a single-backend persistence layer with explicit transactions and
typed repositories.

### Task 2.1 — Design the schema from the new evidence model

Files:

- `src/store/schema.sql`
- `src/store/migrations.rs`
- `src/store/mod.rs`

Tables should cover:

- project metadata;
- revisions;
- files;
- file content or content references;
- evidence records;
- imports;
- dependency edges;
- AI-authored findings;
- schema metadata.

Do not create a risk-pattern table. Do not create a table whose purpose is to
claim that SlugAudit found a bug.

Validation:

1. Apply the schema to a fresh temporary SQLite database.
2. Apply it twice and prove migration is idempotent.
3. Verify foreign keys are enabled.
4. Verify indexes exist for path, hash, revision, symbol name, and dependency
   lookups.
5. Verify the schema has no `risk_pattern` or equivalent automated-finding
   table.

### Task 2.2 — Implement connection and transaction boundaries

Files:

- `src/store/connection.rs`
- `src/store/transaction.rs`

Actions:

1. Open SQLite with explicit busy timeout.
2. Enable foreign keys on every connection.
3. Apply the explicitly documented journal, synchronous, busy-timeout,
   checkpoint, and filesystem policy for the per-project database.
4. Expose transaction helpers that commit or roll back deterministically.
5. Ensure a failed write cannot leave a connection in a poisoned transaction.

Validation:

1. Test rollback after a constraint failure.
2. Test that a subsequent transaction succeeds after rollback.
3. Test concurrent reader behavior during a write transaction.
4. Test reopening the database after a simulated process interruption.
5. Run `cargo nextest run store`.

### Task 2.3 — Implement typed repositories

Files:

- `src/store/files.rs`
- `src/store/evidence.rs`
- `src/store/relationships.rs`
- `src/store/findings.rs`
- `src/store/revisions.rs`

Rules:

1. Repositories accept typed request values.
2. Repositories return typed records or typed errors.
3. SQL stays inside store modules.
4. No handler constructs SQL.
5. Bulk replacement is transactional.

Validation:

1. Insert and retrieve one file.
2. Replace all derived evidence for a file.
3. Delete a file and verify cascading derived records.
4. Insert an AI finding tied to a hash.
5. Change the file hash and verify stale findings are invalidated.
6. Test duplicate and foreign-key violations.
7. Run mutation testing on repository tests once the phase is complete.

## 8. Phase 3 — Implement project discovery and synchronization

Goal: make every query operate on a complete, current snapshot without manual
sync commands.

### Task 3.1 — Validate project roots and activation

Files:

- `src/project/root.rs`
- `src/project/activation.rs`
- `src/project/database_path.rs`

Rules:

1. Resolve the project root canonically.
2. Reject a root that is not a directory.
3. Require `.planning/slugaudit/` for an active project.
4. Derive the database path only inside that directory.
5. Never accept a database path supplied by the AI.
6. Treat creating/removing this directory as the only human-facing action in
   the entire product. No other tool, command, or flag is human-facing.
7. Creating the activation directory immediately starts a full import in the
   background. It is never deferred until an AI makes its first tool call.

Validation:

1. Test valid root.
2. Test missing activation marker.
3. Test file-as-root.
4. Test path traversal attempts.
5. Test database path remains within the project activation directory.

### Task 3.2 — Implement ignore-aware file discovery

Files:

- `src/sync/discovery.rs`
- `src/sync/ignore.rs`
- `src/sync/file_kind.rs`

Rules:

1. Include source, configuration, documentation, scripts, and infrastructure
   files.
2. Exclude binary files from text indexing.
3. Apply the explicitly documented ignore-file names, precedence, and policy
   version; unsupported ignore behavior is reported rather than implied.
4. Exclude the audit database and generated parser cache from project content.
5. Do not follow symlinks outside the project root.
6. Return deterministic path ordering.

Validation:

1. Fixture includes source, YAML, Markdown, shell, binary, ignored, and
   symlink cases.
2. Assert complete expected file set.
3. Assert binary files are classified, not silently dropped.
4. Assert paths are sorted identically across runs.
5. Assert an external symlink is not indexed.
6. Benchmark discovery on a generated fixture.

### Task 3.3 — Implement hashing and manifest comparison

Files:

- `src/sync/hash.rs`
- `src/sync/manifest.rs`

Rules:

1. Hash file bytes, not only timestamps.
2. Record size and hash together.
3. Detect added, modified, unchanged, and deleted files.
4. Treat parser-pack version and extraction contract version as manifest inputs.
5. Never serve a revision whose manifest does not match the current disk state.

Validation:

1. Test unchanged file.
2. Test same-size content change.
3. Test added file.
4. Test deleted file.
5. Test parser version change forces derived evidence refresh.
6. Test a file changing during hashing produces an explicit retry/failure
   outcome instead of a false stable hash.

### Task 3.4 — Implement atomic revision publication

Files:

- `src/sync/revision.rs`
- `src/sync/publish.rs`

Sequence:

1. Discover and hash the complete file set.
2. Compare the manifest to the stored revision.
3. Start a write transaction.
4. Replace changed files and derived evidence.
5. Remove deleted files and related edges.
6. Rebuild affected dependency edges.
7. Insert the new manifest.
8. Mark the revision current only at the final commit.

Validation:

1. End-to-end initial sync fixture.
2. End-to-end modified-file sync.
3. End-to-end deleted-file sync.
4. Inject a failure before commit and verify the old revision remains queryable.
5. Inject a failure after one file update and verify rollback removes partial
   writes.
6. Run two concurrent sync attempts and verify one coherent final revision.
7. Verify no query returns a revision marked current with a mismatched manifest.

## 9. Phase 4 — Integrate the full Tree-sitter language pack

Goal: use the language pack as the extraction system, not as a dependency hidden
behind the old eight-language class hierarchy.

### Task 4.1 — Establish language-pack loading policy

Files:

- `src/parse/registry.rs`
- `src/parse/cache.rs`
- `src/parse/status.rs`

Rules:

1. Detect languages from file paths and content when available.
2. Use the pack’s complete manifest/catalog.
3. Allow on-demand parser downloads according to an explicit policy.
4. Make cache location deterministic and outside indexed project content.
5. Record whether a parser was already cached or downloaded during sync.
6. Expose parser load failures as evidence and operational metadata.

7. Produce a per-language and per-file capability report that separates
   language-pack support from SlugAudit evidence support. The report must show
   detection, parser availability, parse outcome, and each normalized feature
   independently: structures, imports, exports, comments, docstrings, symbols,
   diagnostics, chunks, raw syntax-tree evidence, and source spans. Also report
   the upstream grammar metadata separately: ABI version and availability of
   highlights, injections, locals, indents, folds, and tags. Every unavailable
   feature must carry an omission reason. An empty result is not the same state
   as an unsupported feature or an unavailable parser.

Validation:

1. Verify the registry reports the expected full language count.
2. Load a representative sample from major language families.
3. Load a language not present in the old Python eight-language set.
4. Test cache hit behavior.
5. Test unavailable/offline behavior.
6. Verify parser cache files are not indexed as project files.
7. Verify capability reports distinguish unsupported, unavailable, empty, and
   populated features for languages with different query coverage.

### Task 4.2 — Invoke generic pack intelligence

Files:

- `src/parse/process.rs`
- `src/parse/language.rs`
- `src/evidence/normalize.rs`

Use the pack’s generic processing API for:

- structures;
- imports;
- exports;
- comments;
- docstrings;
- symbols;
- diagnostics;
- syntax-aware chunks;
- metrics.

Rules:

1. Configure all useful neutral evidence fields.
2. Do not discard diagnostics when structure extraction succeeds.
3. Do not classify findings.
4. Preserve raw pack output when normalizing.
5. Normalize only fields needed for stable cross-language queries.

Validation:

1. Parse a Python fixture and assert functions, imports, symbols, comments,
   diagnostics, and chunks where provided.
2. Parse a JavaScript/TypeScript fixture.
3. Parse a language never handled by the Python implementation.
4. Parse malformed source and assert diagnostics are persisted.
5. Parse a non-source config grammar and assert data/structure evidence is not
   falsely labeled as a function or bug.
6. Assert every stored evidence item has a source span or an explicit missing
   span reason.

### Task 4.3 — Add a generic raw-AST fallback evidence record

Files:

- `src/evidence/raw_tree.rs`
- `src/parse/tree.rs`

Purpose: when the pack can parse a language but its higher-level intelligence
does not expose a particular construct, retain enough syntax-tree evidence for
the AI to inspect without rereading the entire file.

Rules:

1. Store bounded node type and span data.
2. Enforce a size limit per file and per response.
3. Do not dump unlimited AST text into every response.
4. Keep raw node evidence separate from normalized definitions.

Validation:

1. Test a language with rich generic output.
2. Test a language with sparse tags.
3. Test node-count and byte-size limits.
4. Test deterministic ordering.
5. Test response truncation metadata.

### Task 4.4 — Remove the old extractor architecture from the Rust design

Rust files must not contain ports of:

- `BaseExtractor`;
- `LANG_MAP`;
- eight extractor classes;
- one hand-maintained node-type dispatch tree per language.

Validation:

1. `rg -n 'BaseExtractor|LANG_MAP|RustExtractor|PythonExtractor' src` returns
   no results.
2. The pack-driven parser tests cover languages outside the old set.
3. The file-size gate passes.
4. Review the dependency graph and verify parsing does not depend on a
   language-specific facade.

## 10. Phase 5 — Build evidence queries and search

Goal: make the evidence useful to an AI without requiring repeated file reads.

None of the capabilities below are separately exposed tools. They are the
schema, indexes, and query-support code that make the `query` tool (Task
7.3) fast and complete: literal/regex search becomes a SQLite FTS5 table
`query` can `SELECT` against, source retrieval becomes a `files` content
column, and evidence lookups become ordinary tables with the right indexes.
Build them as internal capability; the AI reaches all of it through SQL.

### Task 5.1 — Implement bounded literal search

Files:

- `src/search/literal.rs`
- `src/search/context.rs`

Behavior:

1. Case-insensitive substring search.
2. Exact project-relative paths.
3. One-based line numbers.
4. Bounded result count.
5. Bounded context characters.
6. Explicit truncation metadata.

Validation:

1. Test match at first and last line.
2. Test Unicode content.
3. Test multiple matches in one line.
4. Test result cap.
5. Test character cap.
6. Test searching docs/config/scripts as well as source.

### Task 5.2 — Implement constrained regex search

Files:

- `src/search/regex.rs`
- `src/search/limits.rs`

Rules:

1. Reject invalid regexes with a typed error.
2. Bound pattern length.
3. Bound result count and context.
4. Avoid catastrophic backtracking by using Rust’s regular-expression engine.
5. Never execute shell commands or code from a pattern.

Validation:

1. Valid regex match test.
2. Invalid regex test.
3. Pattern-size rejection test.
4. Result-limit test.
5. Large-file performance test.
6. Fuzz bounded search inputs with generated patterns.

### Task 5.3 — Implement source retrieval

Files:

- `src/search/read_file.rs`
- `src/search/path_validation.rs`

Behavior:

1. Read only indexed project-relative paths.
2. Support line bounds.
3. Support total character bounds.
4. Return source hash and revision metadata.
5. Return a clear stale/not-found result when appropriate.

Validation:

1. Path traversal rejection.
2. Absolute path rejection.
3. Missing path behavior.
4. Line-bound behavior.
5. Character truncation behavior.
6. Hash matches the stored source.

### Task 5.4 — Implement evidence retrieval

Files:

- `src/evidence/query.rs`
- `src/evidence/format.rs`

Queries:

- file definitions;
- symbols by name;
- imports and exports;
- parser diagnostics;
- comments/docstrings;
- syntax-aware chunks;
- evidence by source span;
- extraction completeness.

Validation:

1. Every query is revision-scoped.
2. Every query returns freshness metadata.
3. Empty results include a reason/status, not just an empty array.
4. Large evidence result is bounded and marked truncated.
5. Query output contains no automated risk or bug conclusion.

## 11. Phase 6 — Build dependency relationships

Goal: provide useful file relationships without pretending to be a complete
compiler or language-semantic resolver.

Tasks 6.1 and 6.2 build real edge data during sync regardless of tool
surface — that work is unchanged. Task 6.3's dependents/dependencies lookup
is reachable through the `query` tool as a recursive CTE over the edges
table, not through a separate `audit_dependents` tool.

### Task 6.1 — Normalize imports into relationship candidates

Files:

- `src/graph/imports.rs`
- `src/graph/candidates.rs`

Rules:

1. Preserve raw import text/path.
2. Store the parser-provided source module.
3. Store whether resolution is exact, heuristic, external, or unavailable.
4. Never turn an unresolved external import into a false local edge.

Validation:

1. Exact relative import test.
2. External package test.
3. Unresolved import test.
4. Ambiguous candidate test.
5. Verify raw import evidence remains available when resolution fails.

### Task 6.2 — Resolve project-local file edges

Files:

- `src/graph/resolve.rs`
- `src/graph/edges.rs`

Rules:

1. Use project-relative paths.
2. Apply bounded extension/index-file resolution.
3. Record the resolution method.
4. Do not implement language-specific semantic resolution unless an evidence
   adapter is later justified and isolated.

Validation:

1. Relative path resolution.
2. Index/module file resolution.
3. Extension candidate resolution.
4. Ambiguity produces no false edge.
5. Deleted target removes the edge.
6. Circular imports do not recurse indefinitely.

### Task 6.3 — Implement dependents/dependencies queries

Files:

- `src/graph/query.rs`

Validation:

1. Incoming query returns all dependent files.
2. Outgoing query returns imported files.
3. Missing path returns a typed empty/not-found result.
4. Edge results are revision-scoped.
5. Graph query is bounded.
6. Cycle fixture returns finite deterministic output.

## 12. Phase 7 — Implement MCP transport and tools

Goal: expose the evidence system to the AI over stdio without leaking protocol
or logging data onto stdout.

### Task 7.1 — Select and pin the MCP Rust SDK

Files:

- `Cargo.toml`
- `Cargo.lock`
- `.planning/DEPENDENCIES.md`

Actions:

1. Select the maintained Rust MCP SDK compatible with Rust 1.97.1.
2. Verify stdio transport support.
3. Verify tool schema support.
4. Verify error response support.
5. Record the selected version and reason.

Validation:

1. Minimal initialize handshake test.
2. Tool-list response test.
3. Invalid-request test.
4. Confirm no stdout logging is emitted outside protocol frames.
5. Run `cargo audit` and `cargo deny check`.

### Task 7.2 — Implement server lifecycle

Files:

- `src/server.rs`
- `src/main.rs`
- `src/runtime/config.rs`
- `src/runtime/logging.rs`

Rules:

1. `main.rs` only parses startup state and invokes the server.
2. Logs go to stderr.
3. Startup errors are actionable and non-secret.
4. No database is opened until a tool call identifies an active project.
5. No stdout bytes occur except valid MCP protocol output.

Validation:

1. Launch and initialize over a real stdio subprocess.
2. Capture stdout and assert it contains only valid protocol frames.
3. Capture stderr and assert startup diagnostics are present there.
4. Terminate cleanly.
5. Send malformed input and verify bounded failure behavior.

### Task 7.3 — Implement tool modules

Files:

- `src/tools/report.rs`
- `src/tools/query.rs`
- `src/tools/structure.rs`
- `src/tools/finding.rs`

Tool contracts:

- **`report`** — automatic project snapshot, no query authoring required:
  file/language counts, parser availability and failures, evidence-kind
  counts, dependency graph status, open AI-authored findings, evidence
  limitations. Absorbs what an earlier draft called `overview`/`brief`/
  `file_tree` into one call.
- **`query`** — arbitrary read-only SQL executed directly against the active
  project's own SQLite database file. This is the general-purpose tool:
  literal/regex search, symbol/import/export/diagnostic lookup, dependency
  traversal (recursive CTEs over the edge table), and source retrieval (a
  `files` table content column) are all reachable through it. There is no
  query-text validation, keyword blocklist, or table allowlist — safety
  comes from the connection, not from parsing intent out of SQL text (see
  rule 3).
- **`structure`** — Tree-sitter structural pattern matching: runs a
  tree-sitter query (the same S-expression query language grammars ship
  `.scm` files for — highlights/locals/tags syntax, generalized to
  arbitrary caller-supplied patterns) against a file's syntax tree. For
  patterns normalized evidence and `query` can't easily express — e.g.
  "every empty catch block," "every function missing a return type."
  Bounded the same way as the raw-AST fallback (Task 4.3): per-file/
  per-response node-count and byte-size limits, with truncation metadata.
- **`finding`** — the one write. Persists an AI-supplied conclusion (path,
  source hash, line range, severity/category/description) tied to a real
  current file hash; never generates one; auto-invalidated the moment that
  file's hash changes.

Rules:

1. Each tool validates its request using typed input structs.
2. Each tool invokes project freshness verification before querying. If the
   activation-triggered import (Task 3.1) is still running, the call waits
   on that same in-flight sync rather than starting a redundant one; it never
   proceeds against a partial or pre-import state.
3. `query` opens a dedicated connection with `SQLITE_OPEN_READ_ONLY`. A
   write statement fails at the SQLite engine level regardless of query
   text — not because an app-level check rejected a keyword. There is no
   other project's data reachable from that connection, because each
   project already owns exactly one database file.
4. `query` wraps every execution with a hard row cap (e.g. `SELECT * FROM
   (<query>) LIMIT N`) and a query-text length limit; it never returns an
   unbounded result set regardless of what the caller asks for.
5. `structure` enforces the same node-count/byte-size limits as the raw-AST
   fallback evidence (Task 4.3).
6. `finding` is the only tool that writes anything; the other three are
   strictly read-only.
7. Each tool rejects unknown or out-of-contract arguments.
8. No tool exists to manually trigger synchronization. Freshness
   verification in rule 2 is the only entry point; there is no separate
   sync/rebuild tool for a human or an AI to call.
9. Every tool response is shaped for AI consumption only — compact, bounded,
   deterministic — with no human-readability formatting requirement.

Validation for every tool:

1. Valid request test.
2. Missing required field test.
3. Invalid path or limit test.
4. Stale revision test.
5. Empty result test.
6. Large result/truncation test.
7. End-to-end MCP invocation test.

Validation specific to `query`:

1. Attempt a write statement (`INSERT`/`UPDATE`/`DELETE`/`DROP`/`ATTACH`)
   through `query` and confirm it fails at the connection level, with no
   app-side keyword/text rejection involved.
2. Confirm a pathological query is still bounded by the row cap.
3. Confirm arbitrary valid read SQL (joins, CTEs, window functions) succeeds
   — the tool must not impose the old single-table/no-join restriction.

## 13. Phase 8 — Findings and freshness lifecycle

Goal: preserve only conclusions the AI explicitly reviewed, and remove them
when their source evidence changes.

### Task 8.1 — Store AI-reviewed findings

Files:

- `src/store/findings.rs`
- `src/tools/finding.rs`

Fields:

- path;
- source hash;
- line range;
- severity supplied by the AI;
- category supplied by the AI;
- title;
- description;
- created timestamp;
- evidence revision.

Validation:

1. Persist a finding against a real file hash.
2. Retrieve it through `report` (or via `query` directly against the
   findings table).
3. Modify the file and sync.
4. Verify the old finding is not returned as current.
5. Verify SlugAudit never creates a finding during sync.
6. Verify no risk-pattern extraction code exists.

### Task 8.2 — Implement the report tool's inventory

Files:

- `src/tools/report.rs`
- `src/evidence/inventory.rs`

The report must include:

- file count;
- language count;
- parser availability;
- parser failures;
- definition/symbol/import/export counts;
- diagnostic count;
- chunk availability;
- dependency graph status;
- open AI-authored findings;
- evidence limitations.

It must not report:

- risk leads;
- suspicious patterns;
- automated severity;
- an audit score;
- a claim that the project is safe or unsafe.

Validation:

1. Fixture with successful parses.
2. Fixture with unsupported/non-source files.
3. Fixture with parser failure.
4. Fixture with an AI finding.
5. Assert prohibited judgment fields are absent.

## 14. Phase 9 — Performance and concurrency

Goal: make deep evidence cheap enough that an AI can use it repeatedly.

### Task 9.1 — Parallelize file parsing safely

Files:

- `src/sync/parallel.rs`
- `src/parse/worker.rs`

Rules:

1. Parsing work may run in parallel.
2. SQLite writes are coordinated through one explicit writer boundary.
3. Parser cache access follows the language-pack contract.
4. Results are sorted before persistence to remain deterministic.
5. One file failure does not silently erase other file results.

Validation:

1. Compare sequential and parallel outputs byte-for-byte after normalization.
2. Run a fixture with at least 100 files.
3. Run repeated parallel syncs and compare revision contents.
4. Inject parser failures in multiple workers.
5. Check for data races with the standard Rust test/build model.

### Task 9.2 — Add performance benchmarks

Files:

- `benches/discovery.rs`
- `benches/parsing.rs`
- `benches/search.rs`
- `benches/sync.rs`

Measure:

- cold parser load;
- warm parser load;
- first sync;
- unchanged sync;
- changed-file sync;
- search latency;
- evidence retrieval latency;
- SQLite database growth;
- memory use for a large fixture.

Validation:

1. Benchmarks run reproducibly on the same fixture.
2. Unchanged sync does not reparse unchanged files.
3. Search and retrieval remain bounded.
4. Record baseline numbers in `.planning/PERFORMANCE.md`.

## 15. Phase 10 — Adversarial correctness testing

Goal: test where the system is likely to fail in production, not only happy
paths.

### Task 10.1 — Filesystem adversarial tests

Cases:

- symlink outside root;
- file deleted during sync;
- file changed during hash;
- unreadable file;
- very large file;
- invalid UTF-8;
- path with Unicode;
- path with spaces;
- ignored file changes;
- database placed inside the project tree.

Validation: every case has an expected explicit result and no panic.

### Task 10.2 — Parser adversarial tests

Cases:

- malformed source;
- empty source;
- deep nesting;
- generated large source;
- unknown extension;
- grammar download unavailable;
- parser cache corruption;
- language alias;
- data format file;
- mixed-language repository.

Validation: parser status and evidence completeness are correct; no failure is
silently turned into an empty successful parse.

### Task 10.3 — Store adversarial tests

Cases:

- transaction constraint failure;
- interrupted commit simulation;
- duplicate path;
- stale revision query;
- stale finding;
- orphan edge;
- database lock contention;
- migration from an older Rust schema.

Validation: database remains structurally valid and old committed data is not
lost on a failed transaction.

### Task 10.4 — MCP adversarial tests

Cases:

- malformed JSON;
- unknown tool;
- unknown field;
- missing project activation;
- invalid path;
- excessive search limit;
- excessive read size;
- parser failure response;
- unexpected internal error.

Validation: every case produces a bounded protocol response, no panic, and no
stdout corruption.

## 16. Phase 11 — Quality automation and CI

Goal: make the standards impossible to forget.

### Task 11.1 — Add CI workflow

Files:

- `.github/workflows/rust.yml`

Required jobs:

1. format;
2. check;
3. clippy with warnings denied;
4. unit and integration tests;
5. nextest;
6. coverage report;
7. source-size/no-unsafe gate;
8. cargo audit;
9. cargo deny;
10. dependency/lockfile consistency.

Validation:

1. Run each command locally with Rust 1.97.1.
2. Make a temporary formatting violation and verify the format job fails.
3. Make a temporary 200-line production file and verify the structural job
   fails.
4. Make a temporary unsafe block and verify the build fails.
5. Remove all temporary violations.

### Task 11.2 — Add dependency policy

Files:

- `deny.toml`
- `.planning/DEPENDENCIES.md`

Policy:

- deny unknown licenses unless reviewed;
- deny known vulnerabilities at the project’s chosen severity threshold;
- document duplicate dependency exceptions;
- document native/FFI dependencies;
- distinguish direct SlugAudit code from transitive unsafe code;
- update dependencies only with the pinned compiler and full validation.

Validation:

1. `cargo deny check` passes.
2. Add a temporary invalid license rule and prove the check fails.
3. Restore the policy and rerun.

## 17. Phase 12 — End-to-end acceptance

Goal: prove the tool works as an AI-facing evidence service, not merely as a
collection of passing units.

### Task 12.1 — Build a representative fixture repository

Fixture contents:

- at least five programming languages;
- one language outside the old eight;
- configuration files;
- documentation;
- scripts;
- intentionally malformed source;
- local and external imports;
- circular imports;
- deleted/modified files during test scenarios;
- an AI-authored finding.

Validation: fixture itself is reviewed and its expected evidence manifest is
checked in.

### Task 12.2 — Run the real MCP workflow

Sequence:

1. Activate the fixture project and confirm the import starts immediately,
   without any tool call.
2. Start the Rust server over stdio.
3. Initialize the MCP session.
4. Call `report`.
5. Use `query` to search for a known symbol.
6. Use `query` to read the relevant source span.
7. Use `query` to traverse dependencies and dependents (recursive CTE).
8. Use `query` to inspect parser diagnostics.
9. Use `structure` to match a tree-sitter pattern the normalized evidence
   doesn't cover.
10. Attempt a write through `query` and confirm it fails at the connection
    level.
11. Persist an AI-reviewed finding via `finding`.
12. Modify the source.
13. Call another tool.
14. Verify the finding is stale and the evidence revision changed.

Validation:

1. Capture the complete JSON exchange without secrets.
2. Assert all responses contain verified freshness metadata.
3. Assert all requested evidence is bounded.
4. Assert no response contains automated risk leads.
5. Assert no response claims to have performed the audit.
6. Assert stdout contains only MCP frames.
7. Assert stderr contains useful operational logs.

### Task 12.3 — Run the complete release gate

Commands:

```bash
rustup show active-toolchain
cargo fmt --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-targets
cargo test --workspace --doc
cargo llvm-cov nextest --workspace --all-features --lcov --output-path lcov.info
cargo audit
cargo deny check
cargo geiger --all-features
tools/check_source_limits.sh
git diff --check
```

Validation: every command exits zero. Any skipped integration test must be
reported with the reason and must not be described as passing.

## 18. Phase 13 — Documentation and handoff

Files:

- `README.md`
- `ARCHITECTURE.md`
- `.planning/IMPLEMENTATION_PLAN.md`
- `.planning/RELEASE_CHECKLIST.md`
- `.planning/PERFORMANCE.md`
- `.planning/DEPENDENCIES.md`

Document:

- installation;
- pinned compiler and edition;
- parser cache behavior;
- offline behavior;
- supported language-pack catalog behavior;
- evidence fields;
- known extraction limitations;
- freshness guarantees;
- SQLite location;
- MCP registration;
- logging behavior;
- test commands;
- no autonomous bug detection boundary;
- deletion of the old Python project after acceptance.

Validation:

1. Follow the README from a clean temporary checkout.
2. Run the server against the fixture repository.
3. Confirm every documented command is accurate.
4. Confirm no documentation says “risk leads,” “automatic bugs,” or “audit
   completed” for behavior the Rust tool does not provide.
5. Run the final release gate again after documentation changes.

## 19. Phase completion rules

No phase is complete until:

1. Every task in the phase has its validation recorded.
2. The fast local gate passes.
3. The file-size gate passes.
4. The no-unsafe source gate passes.
5. Relevant tests pass with `cargo nextest`.
6. New failures are fixed or explicitly documented as blockers.
7. The working tree diff is reviewed for accidental scope expansion.
8. The next phase does not begin while the current phase has an unresolved
   correctness failure.

## 20. Final acceptance criteria

The Rust rewrite is accepted only when all of the following are true:

- Rust 1.97.1 and edition 2024 are used.
- All production source files are under 200 code lines unless a documented,
  reviewer-approved exception proves splitting would worsen the architecture.
- SlugAudit source contains no unsafe Rust.
- Tree-sitter language-pack processing is the extraction foundation.
- A language outside the old eight-language set is parsed and its evidence is
  exposed successfully.
- Every indexed non-binary file is searchable and retrievable.
- Parser failures are visible and not disguised as empty success.
- Sync is current, hash-based, atomic, and tested under failure.
- SQLite is the only backend and has transaction/migration coverage.
- Dependency relationships are bounded and explicit about uncertainty.
- AI findings are only persisted when supplied by the AI.
- No automated bug detector or risk-pattern engine exists.
- MCP stdio output is protocol-pure.
- Logs go to stderr and cover sync, parser, store, and tool boundaries.
- Search and evidence responses are bounded for token-efficient AI use.
- Full tests, coverage, audit, license, clippy, formatting, mutation, and
  structural checks have been run and their results recorded.
- The end-to-end fixture workflow succeeds.
- The user can delete `/opt/slugaudit-mcp` without deleting or weakening the
  Rust implementation.

## 21. Mandatory amendments from plan audit

The following amendments override any earlier wording that is less precise.
They are required before implementation proceeds beyond the baseline.

### 21.1 Baseline enforcement moves to Phase 0

Phase 0 must deliver all of the following before Phase 1:

- a README with build, test, run, and project-boundary instructions;
- the initial module skeleton matching `ARCHITECTURE.md`;
- a baseline unit test, integration test, and fixture project;
- `tools/check_source_limits.sh` enforcing the under-200-line rule and direct
  source no-unsafe rule using parsed/compiler-backed checks rather than a raw
  text grep alone;
- minimal CI for format, check, clippy, tests, source limits, and no unsafe;
- process-level `Result`, exit-code, panic, and error-redaction policy;
- stderr-only structured logging and stdout-purity tests;
- startup and first-sync latency/progress budgets;
- initial dependency, license, and transitive-unsafe inventory;
- a dated plan decision log;
- a complexity baseline report;
- an explicit project license, dependency attribution, and release-provenance
  policy;
- an explicit statement that TUI rendering and keybindings are not applicable
  to this stdio MCP product, while protocol UX remains in scope;
- an MCP handshake/API probe proving the binary is not a successful placeholder.

### 21.2 Typed evidence state model

The model must not use one enum for parser resource state and parse outcome.
Use separate types for parser availability, parse outcome, extraction
completeness, and evidence origin. Source identity, file metadata, language
selection, parser run, and file evidence are separate owned types.

Evidence spans are zero-based, half-open byte ranges with explicitly defined
one-based display lines and column units. Unicode, CRLF, EOF, and invalid UTF-8
behavior must be tested.

Hashes use raw file bytes with a named algorithm and version. Timestamps are
explicit UTC metadata and are advisory; hashes control correctness. Evidence
records have stable IDs, deterministic ordering, deduplication rules,
missing-span provenance, per-file/per-response budgets, and truncation fields.

Freshness is represented by a verified revision capability created only after a
complete manifest comparison. Public response constructors require that
capability rather than accepting arbitrary freshness metadata.

### 21.3 Language-pack contract

The authoritative upstream is `xberg-io/tree-sitter-language-pack`; the pinned
Rust crate/API must be verified by a local compile probe. The pack advertises
306 parsers, on-demand downloads/cache, generic `process()` intelligence,
low-level `get_parser()`, aliases, varying bundled query coverage, and ABI
variation.

The implementation must use `process()` for normalized evidence and use
`get_parser()` only for bounded raw-tree evidence that the normalized API
cannot provide. It must not invent an arbitrary query-execution feature through
`process()`.

Persist a per-language capability matrix; normalize lowercase canonical names
and aliases; distinguish identifiers from references; retain partial structure
alongside recoverable diagnostics; validate ABI representatives; reuse parser
instances per worker/language context; and define cold-cache, warm-cache,
offline, corrupted-cache, prefetch, checksum, permissions, atomic-install,
retry, and cache-location behavior. Parser-cache files must never be indexed as
project content.

### 21.4 SQLite invariants

Before repositories are implemented, define every table’s columns, primary key,
foreign keys, nullability, unique constraints, revision ownership, indexes,
cascade behavior, and checks. Every derived row is revision-scoped and queries
require a verified revision.

The sync/application layer is the sole owner of multi-repository transactions.
Repositories receive a transaction context and never commit independently.
Choose and document SQLite journal, synchronous, busy-timeout, checkpoint,
lock, and filesystem policies, including per-project lock timeout, winner/loser
behavior, and stale-lock recovery.

The first release must choose and document source-content retention, oversized
file behavior, database-growth limits, and FTS5 versus bounded-scan search.
Migrations are numbered, forward-only, transactional, recoverable, and reject
newer unsupported schema versions. Findings have current/stale/history states
and source-hash plus evidence-contract invalidation.

### 21.5 Stable snapshot and query rules

After parsing and before publication, changed files receive a final hash check.
If the hash differs, the file is retried within a bounded limit or recorded as
unstable and excluded from the verified derived revision. Unreadable/vanished
files are not silently treated as deletions.

Every read binds one verified revision for its entire operation. Discovery
outcomes distinguish indexed, excluded, binary, unreadable, and vanished files.

### 21.6 Search and graph rules

Search and graph APIs define Unicode behavior, stable ordering, result and byte
budgets, cancellation, latency targets, and source-versus-structured-evidence
semantics. Import relationships retain raw import text and include resolution
kind, confidence, candidates, ambiguity, provenance, and revision. Unresolved
is not equivalent to external. Graph traversal is finite and bounded.

### 21.7 MCP contract moves earlier

The exact tool list, JSON schemas, initialize response, protocol version,
framing, EOF, cancellation, concurrency, progress, shutdown, and error mapping
must be captured and tested before tool implementation. Internal failures log
to stderr and map to bounded protocol errors; logs and progress never reach
stdout. Each tool requires a verified revision and has a documented response
budget.

### 21.8 Observability and UX gates

Required stderr event families are startup, request, root validation, sync,
discovery, lock wait, parser cache/load/process, normalization, store
transaction, search, graph resolution, finding transition, and shutdown.
Events include correlation ID, project/revision, duration, counts, truncation,
and error class; they exclude source content, secrets, search patterns, and
finding descriptions by default.

The product is not a TUI. Startup latency, first-sync progress,
waiting/syncing/failure/empty-evidence feedback, cancellation, and protocol
purity are the applicable user-experience requirements.

### 21.9 Performance and adversarial gates

Define reproducible startup, cold/warm parser, first-sync, unchanged-sync,
changed-sync, search, evidence-query, memory, database-growth, and MCP latency
budgets. Define fixture size, machine assumptions, backpressure, parser-worker
ownership, deterministic commit ordering, and regression thresholds.

Add a failure matrix mapping each adversarial case to expected database state,
revision state, response, log event, and exit behavior. Add property/fuzz tests
for paths, spans, regex, evidence limits, and protocol frames. Add restart,
process-kill, cache-corruption, resource-limit, redaction, and deterministic
replay tests.

Mutation testing has an explicit acceptance threshold: no surviving mutation is
allowed in freshness, hashing, revision publication, parser-status mapping,
path validation, search limits, transaction rollback, finding invalidation, or
MCP framing. A surviving mutation elsewhere requires a documented review and
an explicit reason it does not represent a meaningful behavior.

### 21.10 Continuous enforcement and acceptance

CI enforcement begins in Phase 0, not Phase 11. CI checks the pinned toolchain,
lockfile, format, check, clippy, tests, nextest, coverage policy, source limits,
no direct unsafe, dependency advisories, licenses, architecture direction, and
documentation/schema/test drift.

The final fixture has a versioned golden manifest and evidence contract. The
acceptance gate asserts exact tool schemas, all non-binary files searchable,
partial language capability disclosure, no automated findings, restart
behavior, latency budgets, and zero skipped critical tests. All limitations and
exceptions are recorded in the decision log. The release also includes the
project license, third-party attribution, pinned lockfile, compiler identity,
build metadata, and a reproducible artifact checksum.

## 22. Codebase audit corrections — 2026-08-11

Status correction: the header line "Status: planned, implementation not yet
started beyond the scaffold" is stale. The Rust implementation is
functionally complete (73 commits, six MCP tools, watcher-backed sync,
coverage-gated at 89%, perf-gated criterion baselines). The phase sections
below describe the plan as executed plus the corrections from the
2026-08-11 codebase audit.

These corrections were identified by a full-source audit of the implemented
codebase (not of this plan's prose). Each is a required change to the code or
an acceptance gate; each closes a defect found during the audit. Order by
severity; C1 and C2 are correctness/performance blockers.

### 22.1 C1 — Fix `extract_reference` keyword-substring ordering (correctness)

- **Where**: `src/graph/resolver/generic.rs::extract_reference`.
- **Defect**: the `trimmed.contains("from")` gate (JS/TS branch) runs before
the Python `import` branch and matches the substring anywhere in the text.
Python modules whose path contains `from` — `import from_utils`,
`import a.from_b`, `from from_utils import x` — fall into the JS extractor,
find no quoted string, and return `None`; the import is dropped from the
dependency graph with no error.
- **Fix**: gate the JS branch on `trimmed.starts_with("import")` AND a
whitespace-delimited `from` token, never a bare substring; reorder so
keyword-prefix branches (`import`, `from`, `using`, `use`, `open`,
`#include`) are matched by language tokenization, not text order. Add
regression tests for module names containing `from`/`use`/`open` in every
branch's language.

### 22.2 C2 — One parse per file across extraction passes (performance)

- **Where**: `src/evidence/normalize.rs::extract`, `extract_bindings`,
`src/evidence/generic_imports.rs::extract_generic_imports`.
- **Defect**: a single file is parsed up to three times — `process()`, the
binding walker's own `get_parser`+`parse`, and (for the ~365 languages the
pack doesn't extract imports for) the generic import walker's own parse.
Triple parsing is the dominant cost of first import and directly compounds
with the 60 s sync deadline.
- **Fix**: parse once per file and share the `Tree` across the bindings and
generic-imports passes; fold both walkers into one tree walk. Gate the
generic import walker's parse on a cheap pre-scan only if a shared-tree
refactor is infeasible.

### 22.3 C3 — Parallelize parsing behind deterministic ordering (revive Task 9.1)

- **Where**: `src/sync/sample.rs::sample_all_with_deadline`,
`benches/sync.rs`.
- **Defect**: Task 9.1 (parallel file parsing) was never implemented;
sampling+analysis is fully serialized per file. With the 60 s
`max_sync_wall_clock` default, a large monorepo first import can hit
`TimeBudgetExceeded` on every tool call and become unusable (the most
likely first production failure — see 22.12).
- **Fix**: parallelize `analyze`/hashing across a bounded worker pool (e.g.
`std::thread::scope` with a fixed worker count derived from CPU count),
collect results, sort by `relative_path` before persistence for
deterministic revisions, and keep the existing per-file deadline checks.
Measure first-import wall clock for a 10k-file fixture against the 60 s
budget and re-baseline `perf_baseline.json`.

### 22.4 C4 — Extract the duplicated publish-and-drain path (maintainability)

- **Where**: `src/sync/manager.rs::ensure_current`.
- **Defect**: the `NeedsVerification | Desynced` arm and the `Unavailable`
arm each call `publish::publish` with near-identical error mapping; the
function also performs corruption recovery, project-row ensure, watcher
registration, revision read, and stamping. Two publish blocks will drift.
- **Fix**: extract one `publish_and_verify(connection, root, state, sink) ->
Result<String, ErrorData>` used by both arms; keep `ensure_current` as a
thin health-branch dispatcher.

### 22.5 C5 — Justify or remove the process-global sync manager (architecture)

- **Where**: `src/tools/context.rs::SYNC_MANAGER` (OnceLock singleton).
- **Defect**: §2.4 forbids global mutable state; the singleton watcher is
process-global, forces the single-active-project reporting model, and
creates cross-test interference (requiring env-locks and a race hook).
- **Fix**: either (a) inject a per-server `SourceSyncManager` into the tool
handlers (composition root in `server.rs`), or (b) keep the singleton with
a recorded exception in `DECISIONS.md` explaining the single-process MCP
model and add a test that pins the "one active project" reporting
semantics. Prefer (a).

### 22.6 C6 — CI: decouple benches from the test step; make mutation gate fail-closed

- **Where**: `.github/workflows/quality.yml`.
- **Defect**: `cargo test --all-targets` executes the four criterion bench
binaries (harness = false), so the "Run tests" step runs every bench and
then `check_performance.sh` runs them again. The mutation step runs with
`continue-on-error: true`, so it can never fail CI.
- **Fix**: (a) run tests with `--lib --bins --tests` (exclude benches) and
leave bench execution to the dedicated performance gate; (b) once the
mutation-survivor baseline is established and triaged, flip
`continue-on-error` to false and gate on the recorded survivor count.

### 22.7 C7 — Validate generic node-kind matching against real grammars (correctness)

- **Where**: `src/evidence/normalize.rs::is_variable_binding`,
`is_field_declaration`, `src/evidence/generic_imports.rs::is_import_statement_kind`.
- **Defect**: node kinds are matched by name (`"assignment"`,
`kind.contains("field_declaration")`, a bare `"import"` entry) with no
check that those names exist in the 371 grammars. `assignment` emits a
Symbol for every Python assignment; `kind.contains(...)` can match
unrelated kinds in an unforeseen grammar.
- **Fix**: enumerate the exact node kinds for a fixed grammar matrix (rust,
python, js/ts, go, c/cpp, swift, kotlin, csharp, dart, julia, php, perl,
ocaml, elixir, ruby, java) and assert no false positives per language in
`generic_imports_tests.rs`/`normalize_tests.rs`. Remove bare `"import"`
and the `contains` matching unless a test proves a grammar needs it.

### 22.8 C8 — Add the documentation/schema/test drift gate (revive plan-audit item 14)

- **Where**: `tools/` (new `check_docs_drift.sh`), CI.
- **Defect**: `ARCHITECTURE.md` references `src/sync/publish_edges.rs`
(actual: `revision_edges.rs`), "30 `*_tests.rs` files" (actual: 41), and
"`--test-threads=4` by default" (untrue); `IMPLEMENTATION_PLAN.md`
headers and Phase 5/9 tasks describe behavior that was never built (FTS
search, parallel parsing) and was never descoped in `DECISIONS.md`. The
plan's own correction block required a drift check; it was never
implemented.
- **Fix**: add a CI script that fails on (a) `ARCHITECTURE.md` module-map
references to nonexistent files, (b) stale `IMPLEMENTATION_PLAN.md`
status headers, and (c) plan phases that list files with no
implementation and no `DECISIONS.md` descope entry. Either implement FTS
(Phase 5) or record the descope decision explicitly.

### 22.9 C9 — Define the query pagination ordering contract (correctness)

- **Where**: `src/tools/query.rs::execute_and_collect`.
- **Defect**: `SELECT * FROM (<sql>) LIMIT 501 OFFSET n` without an `ORDER
BY` gives unstable pages: concurrent publishes shift rows between pages,
so `next_offset` paging can skip or duplicate rows.
- **Fix**: document that results are ordered by the query's own row order
and that paging is only stable while the revision does not change; return
the revision id (already present) so the AI can detect paging across a
revision boundary, and add a test asserting paging behavior across a
revision change.

### 22.10 C10 — Repo hygiene: vendor cruft and dependency policy hardening

- **Where**: repo root `vendor/` (253 MB, 285 crates, untracked),
`deny.toml`.
- **Defect**: `cargo vendor` output including a `tarpaulin-report.html`
inside `vendor/oorandom/` sits in the working tree; nothing references it
(no `.cargo/config.toml`), and it is untracked so it can be accidentally
committed. `deny.toml` has `multiple-versions = "warn"`, so version
proliferation is visible but not a failure.
- **Fix**: delete `vendor/` (or gitignore it with a comment explaining the
offline-build workflow if one is intended), remove the stray coverage
report, and set `multiple-versions = "deny"` with a documented exception
list once the remaining duplicates (notify 8 vs 7-era windows-sys
entries, ring 0.52.6) are triaged.

### 22.11 C11 — Cache the per-edge unresolved classification at write time (performance)

- **Where**: `src/tools/report.rs::build_report`.
- **Defect**: `unsupported_language_unresolved_count` re-runs
`extract_reference` on every Unresolved edge on every `report` call —
O(edges) re-extraction of raw import text per call.
- **Fix**: store the classifier verdict on `dependency_edges` at write time
(in `revision_edges::resolve_and_store`), or compute the count in SQL
triggered by the resolver registry; remove the per-call loop from
`build_report`.

### 22.12 C12 — Production-failure runbook and budgets (reliability)

- **Where**: `.planning/PERFORMANCE.md`, `src/model/limits.rs`.
- **Defect**: three real-world failure modes are unpriced: (a) Linux
inotify watch limits (`fs.inotify.max_user_watches`) on large repos cause
notify queue overflow → `Desynced` → every call full-publishes, which
under the 60 s budget can hard-fail large repos; (b) a single
pathological file's tree-sitter parse is not interruptible mid-parse (an
accepted limitation, but it can consume the whole budget); (c) macOS
`stat -f` / Windows `fsutil` detection failures fail closed into
`NetworkFilesystemCheck`, which surfaces as an opaque error.
- **Fix**: (a) document the inotify limit and add a `health`-visible
metric for consecutive full publishes; (b) document the single-file parse
ceiling and consider raising `max_sync_wall_clock` for first imports
after C3 lands; (c) surface `NetworkFilesystemCheck` with the underlying
command error and a non-admin fallback message. Record all three in the
runbook section of `PERFORMANCE.md`.

### 22.13 C13 — Logging/tracing coverage closure (observability)

- **Where**: `src/server_runner.rs`, `src/sync/publish*.rs`.
- **Defect**: per-call spans and publish/finding events exist, but there
are no trace events for watcher event volume, per-phase sync durations on
the incremental path, parser load/cache outcomes, or the corruption
recovery path (only warn sites). `OBSERVABILITY.md`'s redaction rules are
documented but not machine-checked.
- **Fix**: add `debug!`/`trace!` events for watcher event counts per
reconcile, per-phase durations, and parser load outcomes; add a test (in
the style of `store/test_capture.rs`) that asserts a future tool call
logging path cannot log SQL text, finding content, or source content.

### 22.14 Acceptance gate updates

1. `cargo test --all-targets` keeps passing; CI test step switches to
`--lib --bins --tests` (C6).
2. New regression tests: `from`-substring module names (C1), pagination
across revision changes (C9), grammar-matrix binding extraction (C7).
3. First-import 10k-file wall-clock benchmark recorded in
`PERFORMANCE.md` and gated (C3).
4. `multiple-versions = "deny"` with exceptions (C10).
5. Drift check green (C8).
