# Connecting SlugAudit to Claude Code

## One-line setup

```bash
slugaudit-mcp connect claude
```

That's it. It writes the `slugaudit` stdio MCP server into your user-level
Claude Code config (`~/.claude.json`). Verify:

```bash
claude mcp list
# slugaudit: /path/to/slugaudit-mcp - ✔ Connected
```

## What you get

Once connected *and* a project is enabled (see below), Claude Code gains
six tools:

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. The real workhorse — joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. No score, no risk leads. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records a conclusion the AI has personally reviewed, bound to the file's hash. |
| `project_control` | Enable/disable SlugAudit for a project (`action = "on"` / `"off"`). |
| `health` | Operational snapshot: watcher health, unreconciled counts, last-sync timestamp. |

SlugAudit itself never audits — it supplies evidence, and the AI does all
the judging.

## Enable a project

Connecting the MCP server makes the tools *available*. To actually index a
codebase, have the agent call the `project_control` tool with
`action = "on"` (optionally with a project path). This creates the
activation marker and SQLite database under `.planning/slugaudit/` inside
the project and runs the first import immediately. After that, Claude
Code can query it.

You only enable once per project. Subsequent Claude Code sessions pick it
up automatically — every tool call re-verifies freshness and waits on any
in-flight import before executing.

## Re-running `connect`

Safe to re-run. If a `slugaudit` entry already exists, it's removed and
re-added, so upgrading the binary and re-running `connect` always points
at the current executable.

## Manual alternative

If you'd rather not use the `connect` command, add it by hand:

```bash
claude mcp add slugaudit -s user -- $(which slugaudit-mcp)
```

Or for a project-scoped registration (only available when working in that
directory):

```bash
claude mcp add slugaudit -s local -- $(which slugaudit-mcp)
```

## Troubleshooting

- **`claude` not found** — install Claude Code:
  `npm install -g @anthropic-ai/claude-code`
- **Tools don't appear in a session** — restart Claude Code after running
  `connect`. Already-running sessions won't see a newly registered MCP
  server.
- **"project not enabled"** — have the agent call `project_control` with
  `action = "on"` for the project you're working in.
- **`/mcps` shows slugaudit as disconnected** — the binary path in your
  config may be stale (you moved or uninstalled it). Re-run
  `slugaudit-mcp connect claude` to refresh.
