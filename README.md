# SlugAudit MCP Rust

SlugAudit supplies an AI with searchable, trustworthy evidence about a codebase.
It does not decide whether code is buggy, assign severity, or replace the AI's
reasoning.

This directory is the new Rust implementation. The older Python checkout is
reference material only and is not a runtime dependency.

## Human interface

SlugAudit has exactly one human-facing control: turning it on or off for a
project. Every other behavior is automatic and reachable only by an AI client
through MCP tool calls — never directly by a human, and never through a
manually-invoked command.

- Enabling or disabling a project is the only action a human ever takes.
- Enabling a project starts a full import immediately, without waiting for an
  AI to ask anything. Every tool call also re-verifies freshness before
  executing, regardless of whether it arrives during, after, or race-adjacent
  to that initial import. There is no manual sync, rebuild, or maintenance
  command for a human or an AI to invoke — the AI never has to trigger, wait
  for, or think about synchronization itself.
- Tool responses are AI-consumed data, not formatted for human reading. There
  is no human-readability requirement on evidence shape — only completeness,
  boundedness, and query-ability matter.
- Once a project is enabled, the AI queries the database for files, symbols,
  imports, and dependencies. It never needs to read a source file directly to
  get that information.

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

The pinned toolchain is read from `rust-toolchain.toml`:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tools/check_source_limits.sh
cargo deny check advisories bans sources
cargo deny check licenses
cargo audit
```

`cargo deny check advisories bans sources` and `cargo deny check licenses`
both pass as of this writing — `Cargo.toml` carries explicit license
metadata (`PolyForm-Noncommercial-1.0.0`), so the license check is no longer
blocked. `cargo audit` reports zero known vulnerabilities across the
dependency tree. Native or transitive unsafe code in third-party crates is
reported separately (e.g. `cargo geiger`, not yet wired into CI); this
project will not add direct unsafe Rust.

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
