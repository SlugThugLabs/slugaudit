# SlugAudit

SlugAudit is an MCP server that gives an AI (Claude, Grok, Codex, or any MCP
client) pre-parsed, searchable, 100%-in-sync evidence about a codebase —
symbols, imports, dependency edges, source spans — so the AI queries a
SQLite database instead of reading hundreds of flat files one at a time.

The saved token budget goes toward deep reasoning (spotting near-duplicate
variable names, subtle structural near-misses, etc.) instead of mundane
file-gathering.

## What you get

Four MCP tools, exposed over stdio:

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. Joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records an audit finding against the evidence. |

## Quick start

```bash
# Build and install
cargo build --release
./target/release/slugaudit-mcp install      # copies to ~/.slugthug/bin/

# Connect your AI agent (Claude Code, Grok, or Codex)
./target/release/slugaudit-mcp connect

# Enable a project (indexes it and runs the first import)
./target/release/slugaudit-mcp enable /path/to/your-project
```

## Documentation

- **[Connecting to your AI agent](docs/README.md)** — `connect` command,
  per-agent guides (Claude Code, Grok, Codex), troubleshooting.
- **[Architecture & build docs](.planning/README.md)** — design decisions,
  implementation plan, how to build from source.

## Design principles

- **Evidence only.** SlugAudit surfaces what's in the codebase. It does not
  decide whether code is buggy, assign severity, or replace the AI's
  reasoning.
- **Per-project SQLite.** Each enabled project gets its own
  `.slugaudit/project.db`. Zero config by default; optional shared
  PostgreSQL for teams.
- **Always in sync.** Every tool call re-verifies freshness and waits on
  any in-flight import before executing — never answers from partial state.
- **Resource-bounded.** File size, query steps, wall clock, and response
  size are all capped. `#![forbid(unsafe_code)]` at the crate root.
