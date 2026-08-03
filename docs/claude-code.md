# Connecting SlugAudit to Claude Code

## One-line setup

```bash
slugaudit-mcp-rust connect claude
```

That's it. It writes the `slugaudit` stdio MCP server into your user-level
Claude Code config (`~/.claude.json`). Verify:

```bash
claude mcp list
# slugaudit: /path/to/slugaudit-mcp-rust - ✔ Connected
```

## What you get

Once connected *and* a project is enabled (see below), Claude Code gains
four tools:

| Tool | What it does |
|------|-------------|
| `query` | Arbitrary read-only SQL against the project's SQLite index. The real workhorse — joins, CTEs, the lot. Row-capped for safety. |
| `report` | Automatic snapshot of the current revision: file counts, languages, what changed since last import. |
| `structure` | Tree-sitter structural pattern matching across 300+ languages. |
| `finding` | The one write tool — records an audit finding against the evidence. |

## Enable a project

Connecting the MCP server makes the tools *available*. To actually index a
codebase:

```bash
slugaudit-mcp-rust enable /path/to/your-project
```

This creates `.slugaudit/` inside the project (the activation marker +
SQLite database) and runs the first import immediately. After that,
Claude Code can query it.

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
claude mcp add slugaudit -s user -- $(which slugaudit-mcp-rust)
```

Or for a project-scoped registration (only available when working in that
directory):

```bash
claude mcp add slugaudit -s local -- $(which slugaudit-mcp-rust)
```

## PostgreSQL / shared index

By default SlugAudit uses zero-config SQLite — one `.slugaudit/project.db`
per enabled project. If you want a single shared PostgreSQL index across
multiple developers or machines, create a `config.toml` (see
`config.toml.example` in the repo) and register with the env var:

```bash
claude mcp add slugaudit -s user \
  -e SLUGAUDIT_CONFIG=/path/to/config.toml \
  -- $(which slugaudit-mcp-rust)
```

Most users don't need this. SQLite is the default and the recommended
starting point.

## Troubleshooting

- **`claude` not found** — install Claude Code:
  `npm install -g @anthropic-ai/claude-code`
- **Tools don't appear in a session** — restart Claude Code after running
  `connect`. Already-running sessions won't see a newly registered MCP
  server.
- **"project not enabled"** — run `slugaudit-mcp-rust enable <path>` for
  the project you're working in.
- **`/mcps` shows slugaudit as disconnected** — the binary path in your
  config may be stale (you moved or uninstalled it). Re-run
  `slugaudit-mcp-rust connect claude` to refresh.
