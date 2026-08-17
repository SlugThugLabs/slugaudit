# SlugAudit

SlugAudit is an MCP server that gives an AI (Claude, Grok, Codex, or any MCP
client) pre-parsed, searchable, 100%-in-sync evidence about a codebase —
symbols, imports, dependency edges, source spans — so the AI queries a
SQLite database instead of reading hundreds of flat files one at a time.

The saved token budget goes toward deep reasoning (spotting near-duplicate
variable names, subtle structural near-misses, etc.) instead of mundane
file-gathering.

## What you get

Six MCP tools, exposed over stdio:

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. Joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records an audit finding against the evidence, bound to the file's current hash. |
| `project_control` | Enable/disable a project — `action = "on"` creates the marker and runs the first import; `action = "off"` purges the project database. |
| `health` | Operational snapshot: watcher health, unreconciled counts, tool-call counters, last-sync timestamp. Read-only — never syncs. |

## Quick start

```bash
# Build and install
cargo build --release
./target/release/slugaudit-mcp install      # copies to ~/.slugthug/bin/

# Connect your AI agent (Claude Code, Grok, or Codex)
./target/release/slugaudit-mcp connect
```

Projects are enabled from inside the AI session: call `project_control`
with `action = "on"` (optionally with a project path), and SlugAudit
creates the activation marker and runs the first import immediately.

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
  `.planning/slugaudit/project.db`. Zero config by default.
- **Always in sync.** Every tool call re-verifies freshness and waits on
  any in-flight import before executing — never answers from partial state.
- **Resource-bounded.** File size, query steps, wall clock, and response
  size are all capped. `#![forbid(unsafe_code)]` at the crate root.

## License

SlugAudit is licensed under the
[PolyForm Noncommercial License 1.0.0 with a SlugAudit tool-use additional permission](LICENSE).

> **Use SlugAudit to build anything you want—even paid software. If
> SlugAudit itself becomes part of what you sell or distribute, contact us.**

You may use SlugAudit internally as a development tool to analyze, audit,
develop, test, or maintain any project, including proprietary and commercial
software. The software you work on and SlugAudit's output do not become subject
to SlugAudit's license merely because you used the tool.

Incorporating, embedding, bundling, distributing, reselling, or offering
SlugAudit itself as part of a commercial product or service requires a separate
written commercial license.

Independent developers and small teams are welcome. We may grant no-cost
commercial integration permission when the project provides clear credit to
SlugAudit and Slug Thug Labs. Contact us before shipping so we can understand
what you're building.

**Commercial integration licensing:** admin@slugthuglabs.dev

See [COMMERCIAL.md](COMMERCIAL.md) for practical examples and contact details.

This is a source-available license, not an OSI-approved open-source license.
