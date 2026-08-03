# SlugAudit MCP Rust

SlugAudit supplies an AI with searchable, trustworthy evidence about a codebase.
It does not decide whether code is buggy, assign severity, or replace the AI's
reasoning.

This directory is the new Rust implementation. The older Python checkout is
reference material only and is not a runtime dependency.

## Current state (Phase 0 foundation)

This is early-stage code. The core correctness mechanisms are in place — no
partial states, atomic writes, concurrent-safe publishes, resource-bounded
operations — but the user-facing experience is incomplete.

**What works:**
- Every tool call syncs the project: discovers, hashes, and parses all files,
  then compares against stored state
- Findings are tied to file hashes and auto-invalidate when code changes
- Concurrent reads see consistent revisions; concurrent publishes use CAS
- Resource limits on all operations (files, memory, responses, query steps)
- Four MCP tools: `report`, `query`, `structure`, `finding`
- Dependency-edge resolution (`src/graph/`): captured `Import` evidence is
  resolved into `dependency_edges` rows on every publish. Python (relative
  imports), Rust (`crate::`/`super::`/`self::` paths), and JS/TS (relative
  imports) resolve to real project files when one exists; everything else —
  unrecognized languages, absolute/bare-name imports, unmatched relative
  imports — is recorded as `External`/`Unresolved` rather than dropped, so
  `query`'s recursive-CTE traversal always sees the full picture, resolved
  or not. See "Dependency-edge resolution scope" below for exact boundaries.

**What's not yet implemented:**
- No optimized incremental sync — every tool call re-discovers and
  re-samples every file, even when nothing changed since the last revision
  (the write itself is skipped in that case, but the filesystem walk isn't)
- No performance baseline (initial import, incremental sync, memory,
  database size, or tool latency have not been measured/budgeted)

See `PACKAGING.md` for installation, MCP registration, activation, database
location/permissions, and upgrade/removal documentation. See
`OBSERVABILITY.md` for tracing and operational-failure handling.

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
- **Rust**: only `crate::`/`super::`/`self::` paths resolve, assumed to be
  rooted at a `src/` directory. A `use` path's trailing segments may name
  an item (a function/type/const) rather than a further nested module —
  the resolver tries the full segment chain as a directory path first, then
  progressively shorter prefixes, since there's no semantic information
  available to tell which case applies. `super::`/`self::` resolution is a
  directory-based heuristic (parent/same directory of the importing file),
  not a real module-tree walk, and is always reported at `"Low"`
  confidence for that reason. Multi-crate workspaces aren't specially
  handled.
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
`src/project/activation.rs` walks up looking for it. The only supported way
to create or remove that marker is the binary's own CLI command, exposed
directly to a human — not an MCP tool, not something an AI calls:

```bash
slugaudit-mcp-rust enable [PATH]   # default PATH: .
slugaudit-mcp-rust disable [PATH]  # prompts before deleting; -y/--yes skips it
slugaudit-mcp-rust help
```

`enable` creates the marker *and* runs the project's first import
immediately, before the command returns — an AI's first tool call never
pays that cost, matching the "starts an import immediately, in the
background" description in `ARCHITECTURE.md` (here, "background" means
"before any tool call is possible," not a detached process — `enable` is a
short-lived CLI invocation that exits once the import completes). `disable`
deletes the marker directory, which also deletes the project's database —
every finding and every piece of evidence — so it asks for confirmation
unless `-y`/`--yes` is passed.

Running the binary with no arguments (or `serve`) is unchanged: it starts
the MCP server over stdio, exactly as before. `enable`/`disable` are
separate, synchronous, one-shot invocations of the same binary, not new
MCP tools — a host application embedding this server (an editor extension,
a wrapper script) can still shell out to these same two subcommands rather
than reimplementing marker/database management itself.

**Planned for future versions:**
- Broader dependency resolution: more languages, real module-resolution
  semantics (e.g. Rust workspace/`mod.rs` awareness, JS `node_modules`)
  rather than the syntax-and-file-existence heuristics described above
- Optimized incremental sync (skip unchanged files)
- A richer human interface (e.g. a status/list command) beyond enable/disable

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

## Development

The pinned toolchain is read from `rust-toolchain.toml`. Local development
gates (same commands `.github/workflows/quality.yml` runs, with `--locked`):

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
bash tools/check_source_limits.sh
cargo deny check advisories bans sources licenses
cargo audit
cargo build --release --locked
```

All of the above pass as of this writing — `Cargo.toml` carries explicit
license metadata (`PolyForm-Noncommercial-1.0.0`), so the license check is
not blocked; `cargo audit` reports zero known vulnerabilities across the
222-crate dependency tree.

CI additionally runs `tests/stdio_protocol.rs` (the real subprocess/
JSON-RPC handshake test) as its own named step, a coverage check
(`cargo llvm-cov`, gated at 89% line coverage — real measured coverage is
above 93% as of this writing), and a mutation-testing step scoped to the
CAS/retry/hash correctness surface (`src/sync/revision.rs`,
`src/sync/publish.rs`, `src/sync/hash.rs`, `src/tools/context.rs` —
currently zero surviving mutants on that scope, verified locally; the CI
step itself is `continue-on-error` since a full-crate baseline hasn't been
established).

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

## CI and protocol boundary

GitHub Actions runs the same baseline gates used locally. Future MCP behavior
must keep protocol responses on stdout and diagnostics on stderr. A command
that changes the tool contract, evidence schema, persistence invariants, or
resource limits must add focused tests before it is considered complete.

## Status

The repository is in the Phase 0 foundation stage. The server behavior and
module tree are intentionally not claimed to be complete.
