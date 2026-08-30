# Connecting SlugAudit to Bob

SlugAudit is an MCP server for AI coding agents. It indexes your codebase
once and exposes the facts to Bob as queryable tools — Bob reads only the
files and lines that matter, instead of re-reading the repository to find
where everything is. SlugAudit gathers facts; **Bob does the analysis**.

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

SlugAudit is Bob's repository fact-gathering layer. It handles repetitive
file and code discovery—symbols, calls, imports, structure, diagnostics, and
changes—so Bob can spend its context on actual analysis instead of reading
the same files again. After connection, it should be invisible in normal use.

Once connected, Bob gains seven tools. Bob should enable the current
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
| `health` | Operational snapshot: watcher health, unreconciled counts, last-sync timestamp. |

SlugAudit itself never audits — it supplies evidence, and the AI does all
the judging.

## Normal use

Start a fresh Bob session after connecting and work normally. Bob should
call `project_control` internally on first use, then use `report`, `query`,
and `structure` to narrow the source it needs to read. You should not manage
the database or repeat imports.


Subsequent sessions reuse the disposable index and re-verify freshness,
updating only what changed.

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
- **"project not enabled"** — Bob needs to call `project_control` once for
  the project. This is an integration issue, not normal user setup.
- **`bob mcp list` doesn't show slugaudit** — the binary path in your
  config may be stale (you moved or uninstalled it). Re-run
  `slugaudit-mcp connect bob` to refresh.
