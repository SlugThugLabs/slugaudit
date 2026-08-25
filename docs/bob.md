# Connecting SlugAudit to Bob

## One-line setup

```bash
slugaudit-mcp connect bob
```

That registers the `slugaudit` stdio MCP server globally in Bob's MCP
config (`--scope global`, so it's available in every project). Verify:

```bash
bob mcp list
# slugaudit: /path/to/slugaudit-mcp - connected
```

## What you get

Once connected *and* a project is enabled (see below), Bob gains seven
tools:

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. The real workhorse — joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. No score, no risk leads. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records a conclusion the AI has personally reviewed, bound to the file's hash. |
| `finding_read` | Session-gated finding query — returns only the current agent session's findings (never another session's conclusions). |
| `project_control` | Enable/disable SlugAudit for a project (`action = "on"` / `"off"`). |
| `health` | Operational snapshot: watcher health, unreconciled counts, last-sync timestamp. |

SlugAudit itself never audits — it supplies evidence, and the AI does all
the judging.

## Enable a project

Connecting the MCP server makes the tools *available*. To actually index a
codebase, have the agent call the `project_control` tool with
`action = "on"` (optionally with a project path). This creates the
activation marker and SQLite database under `.planning/slugaudit/` inside
the project and runs the first import immediately. After that, Bob can
query it.

You only enable once per project. Subsequent Bob sessions pick it up
automatically — every tool call re-verifies freshness and waits on any
in-flight import before executing.

## Re-running `connect`

Safe to re-run. If a `slugaudit` entry already exists, it's removed and
re-added, so upgrading the binary and re-running `connect bob` always
points at the current executable.

## Manual alternative

If you'd rather not use the `connect` command, add it by hand:

```bash
bob mcp add slugaudit --scope global -- $(which slugaudit-mcp)
```

## Troubleshooting

- **`bob` not found** — install the Bob CLI first, or run
  `slugaudit-mcp connect` without an agent and pick Bob from the menu.
- **Tools don't appear in a session** — restart Bob after running
  `connect`. Already-running sessions won't see a newly registered MCP
  server.
- **"project not enabled"** — have the agent call `project_control` with
  `action = "on"` for the project you're working in.
- **`bob mcp list` doesn't show slugaudit** — the binary path in your
  config may be stale (you moved or uninstalled it). Re-run
  `slugaudit-mcp connect bob` to refresh.
