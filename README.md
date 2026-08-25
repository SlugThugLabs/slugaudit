# SlugAudit

SlugAudit is an MCP server that gives an AI (Claude, Grok, Codex, or any MCP
client) pre-parsed, searchable, 100%-in-sync evidence about a codebase —
symbols, imports, dependency edges, source spans — so the AI queries a
SQLite database instead of reading hundreds of flat files one at a time.

The saved token budget goes toward deep reasoning (spotting near-duplicate
variable names, subtle structural near-misses, etc.) instead of mundane
file-gathering.

## What you get

Seven MCP tools, exposed over stdio:

> **Every tool call syncs first.** The server automatically brings the
> project database current before answering — incrementally through the
> filesystem watcher when it's healthy, or via a full disk republish when
> it isn't. You never have to ask for a refresh or worry about stale
> evidence; the sync gate runs unconditionally on every state-bearing
> call.

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. Joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records an audit finding against the evidence, bound to the file's current hash. |
| `finding_read` | Session-gated finding query — returns only the current agent session's findings (never another session's conclusions). |
| `project_control` | Enable/disable a project — `action = "on"` creates the marker and runs the first import; `action = "off"` purges the project database. |
| `health` | Operational snapshot: watcher health, unreconciled counts, tool-call counters, last-sync timestamp. Read-only — never syncs. |

## Quick start

```bash
# Build
cargo build --release

# Run the interactive setup menu
./target/release/slugaudit-mcp menu
```

The `menu` walks you through everything: installing the binary to a
stable path, connecting to a supported AI agent (Claude Code, Grok, or
Codex), getting config snippets for other MCP clients, or starting the
server directly for testing.

Once connected, enable a project from inside the AI session: call
`project_control` with `action = "on"` (optionally with a project path),
and SlugAudit creates the activation marker and runs the first import
immediately.

## CLI reference

```
slugaudit-mcp — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit-mcp                    Run the MCP server (stdio transport)
    slugaudit-mcp menu               Interactive setup menu (recommended entry point)
    slugaudit-mcp install            Copy binary to ~/.slugthug/bin/
    slugaudit-mcp version            Print version (also --version, -V)
    slugaudit-mcp help               Show this message (also --help, -h)

The `menu` walks you through installation, connecting to an AI agent
(Claude Code, Grok, Codex), getting config for other MCP clients,
or starting the server for testing.
```

## Documentation

- **[Architecture & build docs](.planning/README.md)** — design decisions,
  implementation plan, how to build from source.

## Design principles

- **Evidence only.** SlugAudit surfaces what's in the codebase. It does not
  decide whether code is buggy, assign severity, or replace the AI's
  reasoning.
- **Per-project SQLite.** Each enabled project gets its own
  `.planning/slugaudit/project.db`. Zero config by default.
- **Always in sync.** Before any tool answers, the server runs an automatic
  sync pass against the project. If the filesystem watcher stayed healthy,
  it incrementally reconciles whatever changed since the last call (fast).
  If the watcher fell behind or the server just started, it does a full
  publish from disk (correct). Either way you get evidence from the file
  system as it is right now — never a stale view and never partial state
  from a half-finished publish. See [ARCHITECTURE.md](ARCHITECTURE.md) for
  the watcher health state machine.
- **Resource-bounded.** File size, query steps, wall clock, and response
  size are all capped. `#![forbid(unsafe_code)]` at the crate root.
- **Disposable database.** The index is derived from source files and
  reproducible on demand. If the database is corrupt, it is discarded
  and rebuilt. If you want to start fresh, delete
  `.planning/slugaudit/` — the next tool call rebuilds everything.
- **Session-scoped findings.** AI-authored findings are bound to the
  agent session that wrote them. A new agent session starts with a
  clean finding set — it never silently inherits another session's
  audit conclusions.

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
