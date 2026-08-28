# SlugAudit

Searchable, trustworthy codebase evidence for AI agents.

SlugAudit is an MCP server that indexes your project's source files into a
per-project SQLite database and exposes seven tools to any AI agent that
speaks the Model Protocol. It does **not** audit — it supplies evidence, and
the calling AI performs all judgment.

## Product boundary

The end-user product is the single `slugaudit-mcp` binary. Users install and
configure it with their AI agent, and the binary operates on the user's own
projects.

For each enabled user project, SlugAudit owns only this derived-data directory:

```text
<user-project>/.planning/slugaudit/project.db
```

The surrounding `.planning/` directory belongs to the user's project workflow.
Its other files are ordinary user project data and may be indexed normally.
SlugAudit excludes its own `.planning/slugaudit/` directory from discovery.

This repository also contains development-only material used to build and
validate SlugAudit: repository planning documents, tests, benchmarks, CI
workflows, and the `check_*` quality-gate binaries. Those files are not part
of the end-user product or shipped runtime binary.

## Quick start

```bash
# Build the end-user product binary
cargo build --release --locked --bin slugaudit-mcp

# Connect your agent (run once)
./target/release/slugaudit-mcp connect

# Or connect a specific agent directly
./target/release/slugaudit-mcp connect claude
./target/release/slugaudit-mcp connect bob
./target/release/slugaudit-mcp connect grok
./target/release/slugaudit-mcp connect codex
```

Inside your AI session, enable a project:

```
project_control  action="on"  path="/path/to/your/project"
```

That's it. The agent can now query codebase evidence.

## Tools

| Tool | What it does |
|------|-------------|
| `report` | File/language counts, parser failures, evidence kinds, open findings |
| `query` | Read-only SQL against the project's indexed evidence |
| `structure` | Tree-sitter structural pattern matching (300+ languages) |
| `finding` | Persist an AI-reviewed conclusion (auto-invalidates on file change) |
| `finding_read` | Retrieve findings scoped to the current agent session |
| `project_control` | Enable or disable a project |
| `health` | Watcher health, sync status, tool-call counters |

## How it works

1. **Discovery** — walks the project tree, respects `.gitignore`/`.ignore`,
   skips VCS internals and SlugAudit's own data directory.
2. **Sampling** — reads each file, hashes it (BLAKE3), detects language,
   runs tree-sitter extraction (symbols, imports, Rust call sites, comments, diagnostics).
3. **Publishing** — writes everything into `.planning/slugaudit/project.db`
   in one atomic transaction with compare-and-swap concurrency control.
4. **Watching** — a filesystem watcher tracks changes. Incremental reconcile
   re-hashes only dirty files; full re-verification runs when the watcher
   is untrusted.
5. **Querying** — the AI calls `query` with SQL to find exactly which files
   and lines matter, then reads only those.

The database is disposable derived data — delete it and any tool call
rebuilds it from source.

## Repository scope

See [DEVELOPMENT_SCOPE.md](DEVELOPMENT_SCOPE.md) for the distinction between
this repository's development materials and the runtime behavior of SlugAudit
inside a user's project.

## Documentation

- **[Connection guides](docs/)** — agent-specific setup for Bob, Claude Code,
  Grok, and Codex
- **[Architecture](.planning/ARCHITECTURE.md)** — module map, data flow,
  security model, design FAQ
- **[Archive](.planning/archive/)** — historical planning docs, decision log,
  dependency inventory

## Development

Requires Rust 1.97.1+ (edition 2024). Run the same gates CI uses:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib --bins --tests --all-features
cargo run --quiet --bin check_source_limits --locked
cargo run --quiet --bin check_docs_drift --locked
cargo run --quiet --bin check_no_duplicates --locked
```

`#![forbid(unsafe_code)]` is enforced crate-wide. Zero unsafe in `src/`.

## License

PolyForm Noncommercial 1.0.0 — noncommercial use is free; commercial use
requires a separate license from SlugThugLabs. See [LICENSE](LICENSE).
