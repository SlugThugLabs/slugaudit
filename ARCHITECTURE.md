# SlugAudit Rust architecture

This is a new implementation. The Python checkout is reference material only;
its extractor classes and database choices are not being ported automatically.

## Product boundary

SlugAudit collects trustworthy, searchable evidence for an AI code auditor.
It does not detect bugs, assign suspicion, or replace the AI's reasoning.

## Human interface boundary

Everything downstream of project discovery — hashing, parsing, sync,
evidence queries — is automatic and reachable only through MCP tool calls made
by an AI client. A full sync happens on every tool call: every file is
discovered, sampled, and hashed, then compared against stored state. If
nothing has changed since the last revision, the write is skipped and the
current revision is reused; the filesystem sampling still occurs. There is no
enable/disable human interface, no manual sync command, and no background
activation — activation and synchronization are not yet implemented. This is
Phase 0 foundation code; see README.md for current limitations.

Evidence responses are shaped for AI consumption: compact, bounded, and
deterministic. They carry no human-readability requirement — the goal is that
the AI never has to read a source file directly, not that a person can read
the response comfortably.

The AI-facing tool surface is four tools: `report` (automatic project
snapshot), `query` (arbitrary read-only SQL against the project's own
SQLite file — search, symbol/import/diagnostic lookup, dependency traversal,
and source retrieval all reach through this one tool), `structure`
(Tree-sitter structural pattern matching for what normalized evidence
doesn't cover), and `finding` (the one write, an AI-reviewed conclusion).
`query`'s safety comes from a read-only connection and a row cap, not from
parsing or restricting query text — see `IMPLEMENTATION_PLAN.md` Task 7.3.

## The database is a mirror, not a source of truth

`files`/`evidence`/`dependency_edges` exist to save the AI from reading flat
files — they are not where a fix happens. Exactly one code path writes them:
`sync::revision::publish_revision`, reachable only through `sync::publish`,
reachable only through the `ensure_synced` precondition every tool runs
first. `query`'s connection is opened read-only specifically so that path
never opens a second way in, even by accident. `finding` is the sole
exception, and it writes only to the separate `findings` table, which was
never claiming to mirror files.

This isn't defensive hardening against a misbehaving caller — it's what
keeps the mirror trustworthy at all. If the database could drift from disk
through any path other than sync, it would stop being evidence and become a
second, unreliable copy of the truth. A fix is always an edit to the real
file; the next tool call's automatic sync is what brings the database back
in line with it.

## Module ownership

```text
src/main.rs      process entry point only
src/server.rs    MCP transport and server lifecycle
src/tools/       one module per tool: report, query, structure, finding
src/project/     project activation and root validation
src/sync/        discovery, hashing, manifest diff, atomic revision publish
src/parse/       language detection and Tree-sitter pack calls
src/evidence/    neutral extraction and evidence normalization
src/store/       SQLite schema, connections, migrations
src/model/       shared typed records and response metadata
```

There is no separate `src/search/` or `src/graph/` module. Bounded search
folded into `query` (an FTS5/index layer may back it later without changing
the tool contract). Dependency-graph resolution (imports → `dependency_edges`
rows) is not built yet — import evidence is captured but not resolved into
edges.

Each module owns one reason to change. Tool handlers orchestrate; they do not
parse source, build SQL, or format database rows themselves.

## Extraction model

The language pack is the parser and generic intelligence provider. SlugAudit
normalizes its output into stable evidence records: definitions, imports,
exports, symbols, comments, diagnostics, and spans. Syntax-aware chunks and
raw Tree-sitter node information are reserved for future extraction; the
schema and evidence types exist but extraction does not currently generate
them.

No eight-language extractor hierarchy will be recreated. Any language-specific
behavior must be a small, tested adapter justified by an evidence gap.

## Hard quality gates

- Every production Rust file targets fewer than 200 code lines. Code lines
  exclude blank lines, line comments, block comments, and documentation
  comments; executable statements, declarations, macro bodies, and generated
  source are counted according to the enforcement tool's report. A hand-written
  file from 200 through 300 code lines may be approved by the implementation
  agent only with a documented reason showing that splitting it would worsen
  coupling, duplicate logic, or obscure an indivisible state machine. A file
  over 300 code lines blocks implementation and requires the user's approval.
  No 6,000-line class of file is acceptable.
- `#![forbid(unsafe_code)]` applies to SlugAudit source.
- `cargo fmt --check` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all-targets` passes.
- Dependency audit and license checks pass.
- `git diff --check` passes.
- Parser failures are recorded as evidence and never silently converted into
  an apparently complete parse.
- Writes publish one complete revision; readers never observe a partial sync.

Third-party native dependencies may contain internal unsafe code. SlugAudit
will not write or directly expose unsafe Rust; prohibiting unsafe transitively
would make Tree-sitter and SQLite FFI unusable and would be an inaccurate claim.
