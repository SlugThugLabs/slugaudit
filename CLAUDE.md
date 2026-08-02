# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

SlugAudit supplies an AI with searchable, trustworthy *evidence* about a
codebase — it does not decide whether code is buggy, assign severity, or
replace the AI's reasoning. This directory is a from-scratch Rust
implementation; an older Python checkout exists elsewhere as reference
material only and is not a runtime dependency and must not be ported
automatically.

**Status**: Phase 0 foundation. Only `src/model/` (shared typed contracts)
and `src/parse/` (Tree-sitter language-pack boundary) are implemented, plus a
no-op MCP server in `src/server.rs`. `ARCHITECTURE.md` describes the full
target module tree (`tools/`, `project/`, `sync/`, `evidence/`, `store/`,
`search/`, `graph/`) — none of those exist yet. Don't assume they do.

**Human interface**: exactly one control is human-facing — enabling or
disabling SlugAudit for a project. Enabling a project starts a full import
immediately, in the background, without waiting for an AI to make the first
tool call. Every tool call independently re-verifies freshness before
executing — if the activation-triggered import is still running, the call
waits on that same sync rather than starting a redundant one or proceeding
against partial state. There is no manual sync/rebuild/maintenance tool for a
human or an AI to invoke; freshness verification is a mandatory precondition
baked into every other tool call, not a separate step. Tool responses are
shaped for AI consumption — compact, bounded, deterministic — with no
human-readability requirement. Don't add a "sync" tool, a formatting/
pretty-print concern, or any other human-facing surface without checking
with the user first; these have been explicitly ruled out.

**Tool surface**: four tools, per `IMPLEMENTATION_PLAN.md` Task 7.3 —
`report` (automatic snapshot), `query` (arbitrary read-only SQL against the
project's own SQLite file — this is the general-purpose tool; search,
symbol/import/diagnostic lookup, dependency traversal, and source retrieval
all go through it), `structure` (Tree-sitter structural pattern matching for
what normalized evidence doesn't cover), and `finding` (the one write).
`query`'s safety is a read-only connection plus a row cap — never app-level
SQL text parsing/validation; don't reintroduce a keyword blocklist or table
allowlist like the Python original's `validate_sql_query`. Don't add a fifth
tool (e.g. a separate search/read_file/dependents tool) without checking
first — that surface was deliberately collapsed into `query`.

## Commands

Toolchain is pinned in `rust-toolchain.toml` (Rust 1.97.1, edition 2024) —
don't bump it as a side effect of other work. Run the full gate set before
considering anything done; CI (`.github/workflows/quality.yml`) runs the same
sequence:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tools/check_source_limits.sh
cargo deny check advisories bans sources
```

Run a single test: `cargo test <test_name>` or scope by module, e.g.
`cargo test model::span::tests::rejects_reversed_ranges`.

`cargo deny check advisories bans sources` is the current dependency gate;
full license checking (`deny.toml` already declares the allowlist) is
intentionally blocked until `Cargo.toml` carries explicit license metadata.

## Architecture

Planned module ownership (`ARCHITECTURE.md` has the full rationale):

```text
src/main.rs      process entry point only
src/server.rs    MCP transport and server lifecycle
src/tools/       one small module per tool contract        (not yet built)
src/project/     project activation and root validation    (not yet built)
src/sync/        manifest, change detection, revision publish (not yet built)
src/parse/       language detection and Tree-sitter pack calls
src/evidence/    neutral extraction and evidence normalization (not yet built)
src/store/       SQLite schema and repository operations   (not yet built)
src/search/      bounded search and source retrieval        (not yet built)
src/graph/       import/dependency relationship queries     (not yet built)
src/model/       shared typed records and response metadata
```

Each module owns one reason to change. Tool handlers orchestrate — they must
not parse source, build SQL, or format database rows themselves. The parser
layer never writes to SQLite; the store layer never reads project files; the
MCP transport carries no product logic.

**Extraction model**: the Tree-sitter language pack
(`tree-sitter-language-pack`) is the parser and generic-intelligence
provider. SlugAudit normalizes its output into stable evidence records
(definitions, imports, exports, symbols, comments, diagnostics, spans,
syntax-aware chunks) via the types in `src/model/evidence.rs` and
`src/model/parser.rs`. Raw Tree-sitter node info is retained where available.
Parsing failure must always surface as evidence (`ParseOutcome::Failed`,
`ParserAvailability::LoadFailed`, etc.) — never silently reported as a
complete parse. No per-language extractor-class hierarchy; any
language-specific behavior must be a small, tested adapter justified by a
concrete evidence gap.

**Freshness/persistence invariants** (see `src/model/freshness.rs`): writes
publish one complete revision; readers never observe a partial sync. SQLite
is the only store — one database per project, no Postgres/Neo4j.

## Hard constraints (enforced by CI, not just convention)

- `#![forbid(unsafe_code)]` at the crate root (`src/lib.rs`, `src/main.rs`).
  Third-party crates (Tree-sitter, SQLite via `rusqlite`'s `bundled` feature)
  may contain internal unsafe code — that's expected and not a violation.
- **Source-size gate** (`tools/check_source_limits.sh`, Python script invoked
  via bash): production files under `src/` must stay under 200 *code* lines
  (blank lines, line comments, block/doc comments don't count). 200–300 lines
  requires an inline exception comment on its own line:
  `// slugaudit-line-exception: approved-by=agent; reason=<why>`. Above 300
  lines is a hard CI failure requiring a split or explicit user approval —
  don't reach for the exception comment as a way around that.
- No `unwrap()`/`expect()` in production paths unless a documented review
  proves the invariant is local, immutable, and impossible to violate.
- No global mutable state; no hidden network access during an ordinary query
  (an exception is a parser-pack cache miss, and that must be surfaced in
  response metadata, not silent).
- MCP protocol responses stay on stdout; diagnostics go to stderr (see
  `tracing_subscriber` setup in `src/main.rs`, which routes to stderr).
- A change to the tool contract, evidence schema, persistence invariants, or
  resource limits must add focused tests before being considered complete.
