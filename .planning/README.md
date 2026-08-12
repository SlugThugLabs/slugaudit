# SlugAudit MCP Rust

SlugAudit supplies an AI with searchable, trustworthy evidence about a codebase.
It does not decide whether code is buggy, assign severity, or replace the AI's
reasoning.

This directory is the Rust implementation. The older Python checkout is
reference material only and is not a runtime dependency.

## Current state

The core correctness mechanisms are in place and exercised by an end-to-end
test suite: atomic revision publishes, concurrent-safe CAS writes,
watcher-backed incremental sync, resource-bounded operations, and
AI-reviewed findings that auto-invalidate on source change. All quality
gates pass (fmt, clippy `-D warnings`, source-size gate, 343 tests, coverage
≥ 89%).

**What works:**
- **Six MCP tools** — `report` (automatic snapshot), `query` (read-only SQL
  against the project's own database; search, symbol/import/diagnostic
  lookup, and dependency traversal via recursive CTEs all reach through it),
  `structure` (tree-sitter structural pattern matching), `finding` (the one
  write — persists an AI-reviewed conclusion bound to a file hash),
  `project_control` (enable/disable a project), and `health` (operational
  snapshot: watcher health, unreconciled counts, tool-call counters,
  last-sync timestamp).
- **Watcher-backed incremental sync** — `SourceSyncManager` tracks a
  filesystem watcher per project. When the watcher is trusted and events are
  pending, only dirty/deleted paths are re-hashed and re-indexed
  (`reconcile_dirty_paths`); a full publish runs only when the watcher is
  untrusted (restart, integrity violation) or unavailable. The barrier sync
  loop (`sync_with_barrier`, capped at `MAX_BARRIER_LOOPS = 16`) drains
  events that arrive during reconciliation instead of losing them, and
  marks the watcher `Desynced` under a pathological event producer.
- **Atomic, hash-verified revisions** — every publish computes a BLAKE3
  manifest hash and swaps `revisions.is_current` in one transaction;
  concurrent publishers detect the mismatch via compare-and-swap and retry.
- **Findings tied to file hashes** — a finding is stored with the file's
  current `content_hash` and becomes `stale` the moment that hash changes;
  SlugAudit never creates a finding itself.
- **Progress notifications** — `server_runner` emits MCP
  `/notifications/progress` at three wire points per tool call
  (`ensuring_current`, `publishing`, `completed`), plus per-file sampling
  events from the sync layer, so long-running calls are never silent.
- **Dependency-edge resolution** (`src/graph/`) — captured `Import`
  evidence is resolved into `dependency_edges` rows on every publish.
  See "Dependency-edge resolution scope" below for exact boundaries.
- **Resource limits on all operations** — files, memory, responses, query
  steps, and raw evidence are bounded, with truncation metadata when a
  budget is reached.
- **Freshness verification before every answer** — tools bind one verified
  revision for the whole read; a stale handle fails loudly with a retry
  hint rather than serving mismatched data.

**What's not yet implemented:**
- No full-crate mutation-testing baseline (CI mutation is scoped to
  revision/publish/hash/context and `continue-on-error`; the release
  checklist records a dedicated `cargo-mutants` run as pending).
- Startup latency and peak memory have no measured budget yet (noted in
  `.planning/PERFORMANCE.md`); everything else in Task 9.2 has a baseline.

The real-MCP end-to-end workflow (Task 12.2) is **done**:
`tests/stdio_protocol.rs` drives the compiled binary over real stdio
through the whole acceptance sequence — `initialize` → `report` → `query`
(read, then a write attempt rejected at the connection level) →
`structure` → `finding` (persisted `current`) → source modified → finding
flips to `stale` with a second revision — plus restart behavior (a fresh
server serves the same revision from the persisted database).

`health` is genuinely read-only: calling it with a path never syncs,
never registers a watch, and never writes — it reports watcher state (if
registered) and database state (if the database exists) as they already
are, and the response fields document the fix in the decision log.

See `PACKAGING.md` for installation, MCP registration, activation, database
location/permissions, and upgrade/removal documentation. See
`OBSERVABILITY.md` for tracing and operational-failure handling. See
`ARCHITECTURE.md` for the module map and data-flow. See
`.planning/DEPENDENCIES.md` for the dependency policy and inventory,
`.planning/RELEASE_CHECKLIST.md` for the release gate, and
`.planning/DECISIONS.md` for the dated decision log.

### Dependency-edge resolution scope

`src/graph/` resolves what it can prove from syntax and the project's own
known file set — it does not run a real module resolver for any language,
and does not attempt cross-language resolution at all (a Python file calling
into a Rust extension, for instance, is out of scope; it isn't expressible
as a single import statement's `source` text in the first place). Concretely:

- **Python**: only relative imports (`from . import x`, `from ..pkg import
  y`) resolve; absolute imports (`import os`, `from collections import X`)
  are always `External` — without a configured package root there is no
  reliable way to distinguish a project-absolute import from a stdlib/
  third-party one.
- **Rust**: `crate::`/`super::`/`self::` paths resolve against a
  workspace-aware module tree. `crate::` is anchored at the owning crate's
  `src` (walking up to the nearest indexed `Cargo.toml`), so multi-crate
  workspaces work. Glob imports (`use super::*;`) point at the globbed
  module's file, non-`mod.rs` module trees are handled, and trailing item
  names (a function/type/const rather than a further module) fall back to
  progressively shorter segment prefixes. `super::`/`self::` verdicts stay
  `"Low"` confidence because `mod` declarations are not read. Everything
  else (`std::`, bare crate names) is `External`.
- **JavaScript/TypeScript**: only relative imports (`./x`, `../x`) resolve,
  by trying common extensions and directory-index forms against the known
  file set. Bare package names are always `External` — no `node_modules`
  resolution is attempted.
- Every other language, and every import that doesn't match a rule above,
  is recorded as `Unresolved` (if it looked project-relative but nothing
  matched) or `External` (if it looks like a third-party/stdlib reference)
  — the raw import statement text is always preserved either way.

### Activation ownership

A project is "enabled" purely by the presence of a `.planning/slugaudit/`
directory at (or above) the path a tool call is made against —
`src/project/activation.rs` walks up looking for it. The one supported way
to create or remove that marker is the `project_control` MCP tool, exposed
to the calling AI (which a human drives):

- `project_control` with `action = "on"` creates the marker **and** runs
  the project's first import immediately, before the tool returns — an AI's
  later tool calls never pay that cost, matching the "starts an import
  immediately, in the background" description in `ARCHITECTURE.md`.
- `project_control` with `action = "off"` removes the marker and purges the
  project's database — every finding and every piece of evidence — after
  acquiring an exclusive database lock so no concurrent publish can race
  the removal.

The CLI itself (`slugaudit-mcp`) has four commands: `serve` (the MCP server,
the default with no arguments), `connect [AGENT]` (register this binary as
the `slugaudit` MCP server in Claude Code, Grok, or Codex), `install` (copy
the binary to `~/.slugthug/bin/`), and `help`. There is no human-facing
enable/disable CLI command and no manual sync/rebuild command — enabling is
the single human-facing control, everything else flows through MCP tool
calls.

**Planned for future versions:**
- Broader dependency resolution: more languages, real module-resolution
  semantics (e.g. JS `node_modules`) rather than the
  syntax-and-file-existence heuristics described above
- A richer human interface (e.g. a status/list command) beyond the MCP
  tool surface

## Current project rules

- Rust `1.97.1` and edition `2024` are required.
- SlugAudit source must not use `unsafe` Rust. The crate root enforces this
  with `#![forbid(unsafe_code)]`.
- SQLite is the initial store; no PostgreSQL or Neo4j service is required.
- Tree-sitter language-pack support is the parsing foundation. Parsing failure
  must remain visible as evidence rather than being reported as completeness.
- Tool handlers orchestrate. Parsing, evidence normalization, persistence,
  search, and relationship queries remain separate ownership areas.
- The AI performs audit reasoning. Automated extraction must remain neutral.
- The per-project runtime database (`.planning/slugaudit/project.db*`) is
  never versioned — it is gitignored, machine-local, and excluded from
  discovery by `src/sync/discovery.rs` itself.

## Development

The pinned toolchain is read from `rust-toolchain.toml`. Local development
gates (same commands `.github/workflows/quality.yml` runs, with `--locked`):

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo run --quiet --bin check_source_limits --locked
cargo deny check advisories bans sources licenses
cargo audit
cargo build --release --locked
```

All of the above pass as of this writing — `Cargo.toml` carries explicit
license metadata (`PolyForm-Noncommercial-1.0.0`), so the license check is
not blocked; `cargo audit` reports zero known vulnerabilities across the
dependency tree.

CI additionally runs `tests/stdio_protocol.rs` (the real subprocess/
JSON-RPC handshake test, written in Rust) as its own named step, a
coverage check (`cargo run --bin check_coverage --locked` — invokes
`cargo llvm-cov --json` under the hood, then reads the merged JSON line
coverage and prints the gated number, so it can't be misread; threshold
89%), and a mutation-testing step
scoped to the CAS/retry/hash/freshness correctness surface
(`src/sync/revision.rs`, `src/sync/publish*.rs`, `src/sync/hash.rs`,
`src/tools/context.rs`). The mutation step is `continue-on-error` because a
full-crate baseline hasn't been established.

### Unsafe code policy (`cargo geiger`)

`#![forbid(unsafe_code)]` at the crate root means `cargo geiger` reports
`0/0` unsafe usage for `slugaudit-mcp-rust` itself, on every metric
(functions, expressions, impls, traits, methods) — this is a hard compiler
guarantee, not a policy that could silently regress.

Transitive dependencies are a different matter and are expected to contain
unsafe code: `tree-sitter` (native grammar FFI), `rusqlite`'s `bundled`
feature (vendored SQLite C), `tokio` (OS-level async I/O primitives), and
`ring`/`rustls` (cryptography, pulled in transitively by `rmcp`'s transport
layer) all report real unsafe usage under `cargo geiger`, and forbidding
unsafe transitively is not a meaningful goal — it would make every one of
those crates unusable while claiming a guarantee this project doesn't
actually need. The policy is: **zero unsafe in `src/`, enforced by the
compiler; unsafe in a dependency is acceptable when that dependency is
doing FFI, syscalls, or cryptography a pure-Rust implementation can't
avoid, and unacceptable if a dependency uses unsafe for reasons unrelated
to those (a manual review call, not an automated gate)**. `cargo geiger` is
run manually to review the dependency tree when adding a new dependency; it
is not wired into CI as a blocking gate, since geiger's own warning output
is itself noisy (unrelated deprecation/lint warnings from scanning every
transitive crate) and its metric-count output has no natural pass/fail
threshold to gate on — it's a review tool, not a correctness check.

## Source-size gate

Production Rust files under `src/` should contain fewer than 200 code lines.
Blank lines, line comments, block comments, and documentation comments do not
count. A cohesive hand-written file containing 200–300 code lines may carry a
source comment of the form below, with the reason written in the same comment:

```rust
// slugaudit-line-exception: approved-by=agent; reason=indivisible state machine
```

The exception is only valid through 300 code lines. A file above 300 lines is a
hard failure and must be split or brought to the user for approval. Generated
and vendored code is not placed under `src/`; no broad exclusion is permitted.
The rule is enforced in CI and locally by `cargo run --bin check_source_limits
-- locked` — the bin produces the same per-file line counts and the same
PASS/FAIL verdict as the older shell script it replaces.

## CI and protocol boundary

GitHub Actions runs the same baseline gates used locally. Future MCP behavior
must keep protocol responses on stdout and diagnostics on stderr. A command
that changes the tool contract, evidence schema, persistence invariants, or
resource limits must add focused tests before it is considered complete.

## Status

The repository is functional and gate-clean. The performance baseline
(`.planning/PERFORMANCE.md`, Task 9.2) and the
real-MCP end-to-end workflow test (`tests/stdio_protocol.rs`, Task 12.2)
are recorded and passing. The release gate was executed and recorded
on 2026-08-10 (`.planning/RELEASE_CHECKLIST.md`, Task 12.3): every gate
green — fmt, check, clippy `-D warnings`, 332 tests, source limits,
`cargo audit` (0 advisories), `cargo deny` (all ok), coverage 89.32%,
release build sha256 recorded. The license is decided and applied:
`LICENSE` carries the full PolyForm Noncommercial 1.0.0 text — noncommercial
use is free; commercial use requires a separate license from the copyright
holder (SlugThugLabs). Remaining: the scoped mutation baseline and a
startup/memory budget — see the top of this file.
