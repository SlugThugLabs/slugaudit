# Connecting SlugAudit to Claude Code

SlugAudit is an MCP server for AI coding agents. It indexes your codebase
once and exposes the facts to Claude Code as queryable tools — Claude reads
only the files and lines that matter, instead of re-reading the repository
to find where everything is. SlugAudit gathers facts; **Claude Code does the
analysis**.

## One-line setup

```bash
slugaudit connect claude
```

That's it. It writes the `slugaudit` stdio MCP server into your user-level
Claude Code config (`~/.claude.json`). Verify:

```bash
claude mcp list
# slugaudit: /path/to/slugaudit - ✔ Connected
```

## What you get

SlugAudit is Claude Code's repository fact-gathering layer. It handles the
repetitive discovery work—finding files, symbols, calls, imports, structure,
and changes—so Claude can spend more context on analysis instead of reading
the same source repeatedly. After connection, this should be invisible in
normal use.

Once connected, Claude Code gains seven tools. It enables the current
project internally when it first needs the index; you should not need to run
project setup yourself:

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. The real workhorse — joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. No score, no risk leads. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records a conclusion the AI has personally reviewed, bound to the file's hash. |
| `finding_read` | Session-gated finding query — returns only the current agent session's findings (never another session's conclusions). |
| `project_control` | Enable/disable SlugAudit for a project (`action = "on"` / `"off"`). |
| `health` | Operational snapshot: watcher health, unreconciled counts, last-sync timestamp and duration. |

SlugAudit itself never audits — it supplies evidence, and the AI does all
the judging.

## Normal use

Start a fresh Claude Code session after connecting and work normally. Claude
should call `project_control` internally on first use, then use `report`,
`query`, and `structure` to locate the relevant source before reading it.
The goal is not another workflow for you to operate; it is less repetitive
repository reading for Claude.


Subsequent sessions reuse the disposable index and re-verify freshness,
updating only what changed.

## Re-running `connect`

Safe to re-run. If a `slugaudit` entry already exists, it's removed and
re-added, so upgrading the binary and re-running `connect` always points
at the current executable.

## Manual alternative

If you'd rather not use the `connect` command, add it by hand:

```bash
claude mcp add slugaudit -s user -- $(which slugaudit)
```

Or for a project-scoped registration (only available when working in that
directory):

```bash
claude mcp add slugaudit -s local -- $(which slugaudit)
```

## Troubleshooting

- **`claude` not found** — install Claude Code:
  `npm install -g @anthropic-ai/claude-code`
- **Tools don't appear in a session** — restart Claude Code after running
  `connect`. Already-running sessions won't see a newly registered MCP
  server.
- **"project not enabled"** — Claude needs to call `project_control` once
  for the project. This is an integration issue, not normal user setup.
- **`/mcps` shows slugaudit as disconnected** — the binary path in your
  config may be stale (you moved or uninstalled it). Re-run
  `slugaudit connect claude` to refresh.
